use crate::ui;
use eframe::egui;
use p2p_core::{AppCommand, AppEvent};
use tokio::sync::mpsc;

pub struct AppUIState {
    pub show_devices: bool,
    pub show_transfer: bool,
}

impl Default for AppUIState {
    fn default() -> Self {
        Self {
            show_devices: false,
            show_transfer: false,
        }
    }
}

pub struct MyApp {
    // Channels
    cmd_sender: mpsc::UnboundedSender<AppCommand>,
    event_receiver: mpsc::UnboundedReceiver<AppEvent>,

    // App State
    ui_state: AppUIState,

    // Data
    status_log: Vec<String>,
    peers: Vec<String>,
}

impl MyApp {
    pub fn new(
        tx: mpsc::UnboundedSender<AppCommand>,
        rx: mpsc::UnboundedReceiver<AppEvent>,
    ) -> Self {
        Self {
            cmd_sender: tx,
            event_receiver: rx,
            ui_state: AppUIState::default(),
            status_log: Vec::new(),
            peers: Vec::new(),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // 1. Process Events
        while let Ok(event) = self.event_receiver.try_recv() {
            match event {
                AppEvent::Status(msg) => {
                    self.status_log.push(format!("Bot: {}", msg));
                }
                AppEvent::PeerFound { ip, hostname } => {
                    let peer_info = format!("{} ({})", hostname, ip);
                    if !self.peers.contains(&peer_info) {
                        self.peers.push(peer_info);
                    }
                }
                _ => {}
            }
        }

        // 2. Draw Sidebar (Toolbar)
        ui::toolbar::show(ctx, &mut self.ui_state);

        // 3. Draw Central Panel (Playground)
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Playground");
            ui.label("Drag and drop panels here.");
        });

        // 4. Draw Floating Windows
        if self.ui_state.show_devices {
            ui::windows::devices::show(ctx, &mut self.ui_state.show_devices, &self.peers);
        }

        if self.ui_state.show_transfer {
            ui::windows::transfer::show(ctx, &mut self.ui_state.show_transfer);
        }

        // Request repaint for smooth UI
        ctx.request_repaint();
    }
}
