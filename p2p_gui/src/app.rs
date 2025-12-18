use crate::ui;
use eframe::egui;
use p2p_core::{AppCommand, AppEvent};
use std::collections::HashMap;
use std::time::{Duration, Instant};
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

struct PeerInfo {
    ip: String,
    hostname: String,
    last_seen: Instant,
}

pub struct MyApp {
    // Channels
    cmd_sender: mpsc::UnboundedSender<AppCommand>,
    event_receiver: mpsc::UnboundedReceiver<AppEvent>,

    // App State
    ui_state: AppUIState,

    // Data
    status_log: Vec<String>,
    // Key: IP address (unique identifier for now)
    peers: HashMap<String, PeerInfo>,
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
            peers: HashMap::new(),
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
                    // Update or insert peer
                    self.peers.insert(
                        ip.clone(),
                        PeerInfo {
                            ip,
                            hostname,
                            last_seen: Instant::now(),
                        },
                    );
                }
                _ => {}
            }
        }

        // 2. Prune offline peers (older than 12 seconds)
        // Since backend broadcasts every 5s, 12s allows missing 2 packets.
        let now = Instant::now();
        self.peers
            .retain(|_, info| now.duration_since(info.last_seen) < Duration::from_secs(12));

        // Prepare peer list for UI
        let mut peer_list: Vec<String> = self
            .peers
            .values()
            .map(|info| format!("{} ({})", info.hostname, info.ip))
            .collect();
        peer_list.sort();

        // 3. Draw Sidebar (Toolbar)
        ui::toolbar::show(ctx, &mut self.ui_state);

        // 4. Draw Central Panel (Playground)
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Playground");
            ui.label("Drag and drop panels here.");
        });

        // 5. Draw Floating Windows
        if self.ui_state.show_devices {
            ui::windows::devices::show(ctx, &mut self.ui_state.show_devices, &peer_list);
        }

        if self.ui_state.show_transfer {
            ui::windows::transfer::show(ctx, &mut self.ui_state.show_transfer);
        }

        // Request repaint to ensure time-based updates happen even if no events
        // But doing it every frame might be expensive.
        // Instead, request it every second is enough for this UI, but for smooth UI just request it.
        // Or conditionally request if we have peers.
        if !self.peers.is_empty() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }
}
