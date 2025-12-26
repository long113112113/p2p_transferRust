use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use qrcode::QrCode;

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

    // Use the image rendering for actual pixels
    let qr_image = code
        .render::<image::Luma<u8>>()
        .min_dimensions(200, 200)
        .max_dimensions(400, 400)
        .build();

    let width = qr_image.width() as usize;
    let height = qr_image.height() as usize;

    // Convert grayscale to RGBA (each pixel needs 4 bytes: R, G, B, A)
    let rgba: Vec<u8> = qr_image
        .pixels()
        .flat_map(|p| {
            let luma = p.0[0];
            [luma, luma, luma, 255u8]
        })
        .collect();

    Some(ColorImage::from_rgba_unmultiplied([width, height], &rgba))
}

/// Show the QR code window
pub fn show(ctx: &egui::Context, open: &mut bool, cache: &mut QrCodeCache) {
    // Placeholder URL - replace with actual web server URL later
    let target_url = "https://www.youtube.com";

    egui::Window::new("QR Code")
        .open(open)
        .resizable(false)
        .collapsible(false)
        .default_size([300.0, 350.0])
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);

                // Generate or reuse cached texture
                if cache.url != target_url || cache.texture.is_none() {
                    if let Some(image) = generate_qr_image(target_url) {
                        cache.texture = Some(ui.ctx().load_texture(
                            "qr_code",
                            image,
                            TextureOptions::NEAREST,
                        ));
                        cache.url = target_url.to_string();
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
                    ui.monospace(target_url);
                });

                // Copy button
                if ui.button("📋 Copy URL").clicked() {
                    ctx.copy_text(target_url.to_string());
                }
            });
        });
}
