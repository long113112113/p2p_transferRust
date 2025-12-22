use eframe::egui;
use egui_phosphor::regular::{ARROWS_CLOCKWISE, FILE_TEXT, TRASH};

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    download_path: &std::path::Path,
    local_files: &[String],
    refresh_files: impl FnOnce(),
) {
    let mut should_refresh = false;

    egui::Window::new("Files")
        .open(open)
        .resizable(true)
        .default_size([400.0, 300.0])
        .min_size([300.0, 200.0])
        .show(ctx, |ui| {
            ui.heading("File Management");
            ui.add_space(5.0);

            // 1. Show fixed download location (read-only)
            ui.horizontal(|ui| {
                ui.label("Save location:");
            });
            ui.monospace(download_path.to_string_lossy());

            ui.separator();

            // 2. File List
            ui.horizontal(|ui| {
                ui.label(format!("Files in directory ({}):", local_files.len()));
                if ui.button(format!("{} Refresh", ARROWS_CLOCKWISE)).clicked() {
                    should_refresh = true;
                }
            });

            ui.add_space(5.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                if local_files.is_empty() {
                    ui.label(egui::RichText::new("No files found.").italics().weak());
                } else {
                    for file_name in local_files {
                        ui.horizontal(|ui| {
                            ui.label(FILE_TEXT);
                            ui.label(file_name);

                            // Delete button
                            if ui.button(TRASH).on_hover_text("Delete file").clicked() {
                                let file_path = download_path.join(file_name);
                                if let Err(e) = std::fs::remove_file(&file_path) {
                                    eprintln!("Failed to delete file: {}", e);
                                }
                                should_refresh = true;
                            }
                        });
                    }
                }
            });

            if should_refresh {
                refresh_files();
            }
        });
}
