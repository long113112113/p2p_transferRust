use eframe::egui;
use p2p_core::AppCommand;
use tokio::sync::mpsc;

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    peers: &[String],
    cmd_tx: &mpsc::Sender<AppCommand>,
) {
    egui::Window::new("Devices")
        .open(open)
        .resizable(true)
        .default_size([300.0, 200.0])
        .min_size([200.0, 150.0])
        .show(ctx, |ui| {
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
                            let cmd_tx = cmd_tx.clone();
                            let peer_str = peer.clone();

                            // Spawn a thread for file dialog to avoid blocking the UI
                            std::thread::spawn(move || {
                                if let Some(files) = rfd::FileDialog::new().pick_files() {
                                    // Extract IP from "Hostname (IP)"
                                    if let Some(start) = peer_str.rfind('(') {
                                        if let Some(end) = peer_str.rfind(')') {
                                            if start < end {
                                                let ip = peer_str[start + 1..end].to_string();
                                                let name = peer_str[..start].trim().to_string();

                                                let _ =
                                                    cmd_tx.blocking_send(AppCommand::SendFile {
                                                        target_ip: ip,
                                                        target_peer_id: String::new(),
                                                        target_peer_name: name,
                                                        files,
                                                    });
                                            }
                                        }
                                    }
                                }
                            });
                        }
                    });
                }
            }
        });
}
