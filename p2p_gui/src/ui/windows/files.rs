use eframe::egui;
use std::path::PathBuf;

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    download_path: &PathBuf,
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
                if ui.button("🔄 Refresh").clicked() {
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
                            ui.label("📄");
                            ui.label(file_name);
                        });
                    }
                }
            });

            if should_refresh {
                refresh_files();
            }
        });
}
