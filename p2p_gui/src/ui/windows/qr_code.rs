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

/// Show the QR code window with a dynamic URL and server toggle
pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    cache: &mut QrCodeCache,
    url: &str,
    server_running: bool,
    server_pending: &mut bool,
    cmd_sender: &mpsc::Sender<AppCommand>,
) {
    // We need a local mutable copy for the toggle
    let mut toggle_state = server_running;

    egui::Window::new("QR Code")
        .open(open)
        .resizable(false)
        .collapsible(false)
        .default_size([300.0, 400.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
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
                        format!("{} Server ON", egui_phosphor::regular::GLOBE)
                    } else {
                        format!("{} Server OFF", egui_phosphor::regular::GLOBE)
                    };

                    ui.label(status_text);
                    ui.add_space(8.0);

                    // Toggle is disabled when pending
                    let toggle_enabled = !*server_pending;
                    let response = toggle_ui(ui, &mut toggle_state, toggle_enabled);

                    // If toggle was clicked and state changed, send command
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
                    // Generate or reuse cached texture
                    if cache.url != url || cache.texture.is_none() {
                        if let Some(image) = generate_qr_image(url) {
                            cache.texture = Some(ui.ctx().load_texture(
                                "qr_code",
                                image,
                                TextureOptions::NEAREST,
                            ));
                            cache.url = url.to_string();
                        }
                    }

                    // Display QR code
                    if let Some(texture) = &cache.texture {
                        let size = egui::vec2(250.0, 250.0);
                        ui.image((texture.id(), size));
                    } else {
                        ui.label("Failed to generate QR code");
                    }

                    ui.add_space(8.0);
                    ui.separator();

                    // Show the URL
                    ui.horizontal(|ui| {
                        ui.label("URL:");
                        ui.monospace(url);
                    });

                    // Copy button
                    if ui
                        .button(format!("{} Copy URL", egui_phosphor::regular::CLIPBOARD))
                        .clicked()
                    {
                        ctx.copy_text(url.to_string());
                    }
                } else {
                    ui.add_space(40.0);
                    ui.label("Server is not running.");
                    ui.label("Toggle the switch to start the server.");
                    ui.add_space(40.0);
                }
            });
        });
}
