use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use p2p_core::AppCommand;
use qrcode::QrCode;
use tokio::sync::mpsc;

/// Cached QR code texture and the URL it was generated for
pub struct QrCodeCache {
    url: String,
    texture: Option<TextureHandle>,
}

impl Default for QrCodeCache {
    fn default() -> Self {
        Self {
            url: String::new(),
            texture: None,
        }
    }
}

/// Which share mode tab is selected
#[derive(Default, Clone, Copy, PartialEq)]
pub enum ShareTab {
    #[default]
    Lan,
    Wan,
}

/// Generate a QR code image from URL string
fn generate_qr_image(url: &str) -> Option<ColorImage> {
    let code = QrCode::new(url.as_bytes()).ok()?;

    let qr_image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(200, 200)
        .max_dimensions(400, 400)
        .build();

    let width = qr_image.width() as usize;
    let height = qr_image.height() as usize;

    let rgba: Vec<u8> = qr_image
        .pixels()
        .flat_map(|p| {
            let luma = p.0[0];
            [luma, luma, luma, 255u8]
        })
        .collect();

    Some(ColorImage::from_rgba_unmultiplied([width, height], &rgba))
}

/// iOS-style toggle switch widget
fn toggle_ui(ui: &mut egui::Ui, on: &mut bool, enabled: bool) -> egui::Response {
    let desired_size = ui.spacing().interact_size.y * egui::vec2(2.0, 1.0);

    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, mut response) = ui.allocate_exact_size(desired_size, sense);

    if enabled && response.clicked() {
        *on = !*on;
        response.mark_changed();
    }

    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::Checkbox,
            ui.is_enabled() && enabled,
            *on,
            "",
        )
    });

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let visuals = ui.style().interact_selectable(&response, *on);
        let rect = rect.expand(visuals.expansion);
        let radius = 0.5 * rect.height();

        // Background color: green when on, gray when off, dimmed when disabled
        let bg_color = if !enabled {
            egui::Color32::from_gray(80)
        } else if *on {
            egui::Color32::from_rgb(60, 180, 60)
        } else {
            egui::Color32::from_gray(120)
        };

        ui.painter().rect(
            rect,
            radius,
            bg_color,
            visuals.bg_stroke,
            egui::StrokeKind::Inside,
        );

        // Circle position animated from left to right
        let circle_x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let center = egui::pos2(circle_x, rect.center().y);
        let circle_color = if enabled {
            egui::Color32::WHITE
        } else {
            egui::Color32::from_gray(160)
        };
        ui.painter().circle(
            center,
            0.75 * radius,
            circle_color,
            egui::Stroke::new(1.0, egui::Color32::from_gray(200)),
        );
    }

    response
}

/// Show the QR code window with LAN and WAN tabs
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    cache: &mut QrCodeCache,
    selected_tab: &mut ShareTab,
    // LAN share state
    lan_url: &str,
    lan_server_running: bool,
    lan_server_pending: &mut bool,
    // WAN share state
    wan_url: Option<&str>,
    wan_share_running: bool,
    wan_share_pending: &mut bool,
    // Command sender
    cmd_sender: &mpsc::Sender<AppCommand>,
) {
    egui::Window::new("QR Code Share")
        .open(open)
        .resizable(false)
        .collapsible(false)
        .default_size([320.0, 450.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                // Tab selector
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(
                            *selected_tab == ShareTab::Lan,
                            format!("{} LAN", egui_phosphor::regular::WIFI_HIGH),
                        )
                        .clicked()
                    {
                        *selected_tab = ShareTab::Lan;
                        *cache = QrCodeCache::default();
                    }
                    ui.separator();
                    if ui
                        .selectable_label(
                            *selected_tab == ShareTab::Wan,
                            format!("{} WAN", egui_phosphor::regular::GLOBE),
                        )
                        .clicked()
                    {
                        *selected_tab = ShareTab::Wan;
                        *cache = QrCodeCache::default();
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                match selected_tab {
                    ShareTab::Lan => {
                        show_lan_tab(
                            ui,
                            ctx,
                            cache,
                            lan_url,
                            lan_server_running,
                            lan_server_pending,
                            cmd_sender,
                        );
                    }
                    ShareTab::Wan => {
                        show_wan_tab(
                            ui,
                            ctx,
                            cache,
                            wan_url,
                            wan_share_running,
                            wan_share_pending,
                            cmd_sender,
                        );
                    }
                }
            });
        });
}

/// Show LAN share tab content
fn show_lan_tab(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    cache: &mut QrCodeCache,
    url: &str,
    server_running: bool,
    server_pending: &mut bool,
    cmd_sender: &mpsc::Sender<AppCommand>,
) {
    let mut toggle_state = server_running;

    ui.add_space(8.0);

    // Server toggle with label
    ui.horizontal(|ui| {
        let status_text = if *server_pending {
            if server_running {
                format!("{} Stopping...", egui_phosphor::regular::SPINNER)
            } else {
                format!("{} Starting...", egui_phosphor::regular::SPINNER)
            }
        } else if server_running {
            format!("{} LAN Server ON", egui_phosphor::regular::WIFI_HIGH)
        } else {
            format!("{} LAN Server OFF", egui_phosphor::regular::WIFI_SLASH)
        };

        ui.label(status_text);
        ui.add_space(8.0);

        let toggle_enabled = !*server_pending;
        let response = toggle_ui(ui, &mut toggle_state, toggle_enabled);

        if response.changed() {
            *server_pending = true;
            if toggle_state {
                let _ = cmd_sender.try_send(AppCommand::StartHttpServer);
            } else {
                let _ = cmd_sender.try_send(AppCommand::StopHttpServer);
            }
        }
    });

    ui.add_space(8.0);
    ui.separator();

    if server_running {
        show_qr_and_url(ui, ctx, cache, url);
    } else {
        ui.add_space(40.0);
        ui.label("LAN server is not running.");
        ui.label("Toggle the switch to start sharing.");
        ui.add_space(40.0);
    }
}

/// Show WAN share tab content
fn show_wan_tab(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    cache: &mut QrCodeCache,
    wan_url: Option<&str>,
    wan_running: bool,
    wan_pending: &mut bool,
    cmd_sender: &mpsc::Sender<AppCommand>,
) {
    let mut toggle_state = wan_running;

    ui.add_space(8.0);

    // WAN toggle with label
    ui.horizontal(|ui| {
        let status_text = if *wan_pending {
            if wan_running {
                format!("{} Stopping...", egui_phosphor::regular::SPINNER)
            } else {
                format!("{} Connecting...", egui_phosphor::regular::SPINNER)
            }
        } else if wan_running {
            format!("{} WAN Share ON", egui_phosphor::regular::GLOBE)
        } else {
            format!("{} WAN Share OFF", egui_phosphor::regular::GLOBE_X)
        };

        ui.label(status_text);
        ui.add_space(8.0);

        let toggle_enabled = !*wan_pending;
        let response = toggle_ui(ui, &mut toggle_state, toggle_enabled);

        if response.changed() {
            *wan_pending = true;
            if toggle_state {
                let _ = cmd_sender.try_send(AppCommand::StartWanShare);
            } else {
                let _ = cmd_sender.try_send(AppCommand::StopWanShare);
            }
        }
    });

    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Access from anywhere via bore.pub")
            .small()
            .color(egui::Color32::GRAY),
    );

    ui.add_space(8.0);
    ui.separator();

    if wan_running {
        if let Some(url) = wan_url {
            show_qr_and_url(ui, ctx, cache, url);
        } else {
            ui.add_space(40.0);
            ui.label("Waiting for tunnel...");
            ui.add_space(40.0);
        }
    } else {
        ui.add_space(40.0);
        ui.label("WAN share is not active.");
        ui.label("Toggle the switch to create a public URL.");
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new("Note: Uses bore.pub relay (no HTTPS)")
                .small()
                .color(egui::Color32::GRAY),
        );
        ui.add_space(24.0);
    }
}

/// Show QR code and URL with copy button
fn show_qr_and_url(ui: &mut egui::Ui, ctx: &egui::Context, cache: &mut QrCodeCache, url: &str) {
    // Generate or reuse cached texture
    if cache.url != url || cache.texture.is_none() {
        if let Some(image) = generate_qr_image(url) {
            cache.texture = Some(ctx.load_texture("qr_code", image, TextureOptions::NEAREST));
            cache.url = url.to_string();
        }
    }

    // Display QR code
    if let Some(texture) = &cache.texture {
        let size = egui::vec2(220.0, 220.0);
        ui.image((texture.id(), size));
    } else {
        ui.label("Failed to generate QR code");
    }

    ui.add_space(8.0);
    ui.separator();

    // Show the URL (truncated if too long)
    ui.horizontal(|ui| {
        ui.label("URL:");
        let display_url = if url.len() > 35 {
            format!("{}...", &url[..32])
        } else {
            url.to_string()
        };
        ui.monospace(display_url).on_hover_text(url);
    });

    // Copy button
    if ui
        .button(format!("{} Copy URL", egui_phosphor::regular::CLIPBOARD))
        .clicked()
    {
        ctx.copy_text(url.to_string());
    }
}
