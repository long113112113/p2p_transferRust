use eframe::egui;
use egui_phosphor::regular::{COPY, GLOBE, PLUGS_CONNECTED};
use p2p_core::{AppCommand, AppEvent};
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
    cmd_tx: &mpsc::Sender<AppCommand>,
    event_tx: &mpsc::Sender<AppEvent>,
    wan_service: &std::sync::Arc<p2p_wan::ConnectionListener>,
    wan_rt: &tokio::runtime::Handle,
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
                        let target_id_str = state.target_endpoint_id.trim().to_string();
                        state.connection_status = format!("Connecting to {}...", target_id_str);

                        // Send Log command to backend (for consistent logging)
                        let cmd_tx_clone = cmd_tx.clone();
                        let target_id_for_cmd = target_id_str.clone();
                        std::thread::spawn(move || {
                            let _ = cmd_tx_clone.blocking_send(AppCommand::WanConnect {
                                target_endpoint_id: target_id_for_cmd,
                            });
                        });

                        // Perform connection using shared WanService
                        let ws = wan_service.clone();
                        let event_tx = event_tx.clone();
                        let target_id_str = target_id_str.clone();

                        wan_rt.spawn(async move {
                            // 1. Parse Endpoint ID
                            let endpoint_id = match target_id_str.parse::<iroh::EndpointId>() {
                                Ok(id) => id,
                                Err(e) => {
                                    let _ = event_tx
                                        .send(AppEvent::Error(format!(
                                            "Invalid Endpoint ID: {}",
                                            e
                                        )))
                                        .await;
                                    return;
                                }
                            };

                            let _ = event_tx
                                .send(AppEvent::Status(format!(
                                    "Connecting to {}...",
                                    endpoint_id
                                )))
                                .await;

                            // 2. Connect
                            match ws.connect(endpoint_id).await {
                                Ok(connection) => {
                                    let _ = event_tx
                                        .send(AppEvent::Status(format!(
                                            "✓ successfully connected to {}",
                                            connection.remote_id()
                                        )))
                                        .await;
                                    // TODO: Store connection for file transfer
                                }
                                Err(e) => {
                                    let _ = event_tx
                                        .send(AppEvent::Error(format!("Connection failed: {}", e)))
                                        .await;
                                }
                            }
                        });
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
