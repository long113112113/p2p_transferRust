use eframe::egui;

pub fn show(ctx: &egui::Context, open: &mut bool, peers: &[String]) {
    egui::Window::new("Devices")
        .open(open)
        .resizable(true)
        .default_size([300.0, 200.0])
        .min_size([200.0, 150.0])
        .show(ctx, |ui| {
            ui.heading("Nearby Devices");
            ui.label("Devices found on LAN:");
            ui.separator();

            if peers.is_empty() {
                ui.label("Searching...");
            } else {
                for peer in peers {
                    ui.horizontal(|ui| {
                        ui.label("🖥");
                        ui.label(peer);
                        if ui.button("Send Files").clicked() {
                            // TODO: Select this peer for transfer
                        }
                    });
                }
            }
        });
}
