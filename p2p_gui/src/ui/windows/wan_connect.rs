use eframe::egui;
use egui_phosphor::regular::{COPY, GLOBE, PLUGS_CONNECTED};
use p2p_core::AppCommand;
use tokio::sync::mpsc;

pub struct WanConnectState {
    pub target_endpoint_id: String,
    pub my_endpoint_id: String,
    pub connection_status: String,
}

impl Default for WanConnectState {
    fn default() -> Self {
        // Load endpoint ID from Iroh identity
        let my_endpoint_id = p2p_core::identity::get_iroh_endpoint_id();

        Self {
            target_endpoint_id: String::new(),
            my_endpoint_id,
            connection_status: String::new(),
        }
    }
}

pub fn show(
    ctx: &egui::Context,
    open: &mut bool,
    state: &mut WanConnectState,
    _cmd_tx: &mpsc::Sender<AppCommand>,
) {
    egui::Window::new(format!("{} WAN Connect", GLOBE))
        .open(open)
        .resizable(true)
        .default_size([350.0, 200.0])
        .min_size([300.0, 150.0])
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                // My Endpoint ID section
                ui.heading("My Endpoint ID");
                ui.horizontal(|ui| {
                    let id_text = if state.my_endpoint_id.is_empty() {
                        "Loading..."
                    } else {
                        &state.my_endpoint_id
                    };

                    ui.add(
                        egui::TextEdit::singleline(&mut id_text.to_string())
                            .desired_width(250.0)
                            .interactive(false),
                    );

                    if ui
                        .button(format!("{}", COPY))
                        .on_hover_text("Copy to clipboard")
                        .clicked()
                    {
                        ctx.copy_text(state.my_endpoint_id.clone());
                    }
                });

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(12.0);

                // Connect to peer section
                ui.heading("Connect to Peer");
                ui.label("Enter the remote Endpoint ID:");

                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.target_endpoint_id)
                            .desired_width(250.0)
                            .hint_text("Paste Endpoint ID here..."),
                    );

                    let can_connect = !state.target_endpoint_id.trim().is_empty();
                    if ui
                        .add_enabled(
                            can_connect,
                            egui::Button::new(format!("{}", PLUGS_CONNECTED)),
                        )
                        .on_hover_text("Connect")
                        .clicked()
                    {
                        // TODO: Add WAN connect command to AppCommand
                        state.connection_status =
                            format!("Connecting to {}...", state.target_endpoint_id.trim());
                    }
                });

                // Connection status
                if !state.connection_status.is_empty() {
                    ui.add_space(8.0);
                    ui.label(&state.connection_status);
                }
            });
        });
}
