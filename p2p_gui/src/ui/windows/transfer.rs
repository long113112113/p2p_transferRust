use eframe::egui;

pub fn show(ctx: &egui::Context, open: &mut bool) {
    egui::Window::new("Transfers")
        .open(open)
        .resizable(true)
        .default_size([350.0, 250.0])
        .min_size([200.0, 150.0])
        .show(ctx, |ui| {
            ui.label("Transfer status and logs will appear here.");
            ui.separator();
            ui.label("No active transfers.");
        });
}
