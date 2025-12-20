use crate::ui;
use crate::ui::windows::verify::{self, VerificationState};
use eframe::egui;
use p2p_core::{AppCommand, AppEvent};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub struct AppUIState {
    pub show_devices: bool,
    pub show_files: bool,
}

impl Default for AppUIState {
    fn default() -> Self {
        Self {
            show_devices: false,
            show_files: false,
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
    cmd_sender: mpsc::Sender<AppCommand>,
    event_receiver: mpsc::Receiver<AppEvent>,

    // App State
    ui_state: AppUIState,
    verification_state: VerificationState,

    // Data
    status_log: Vec<String>,
    // Key: IP address (unique identifier for now)
    peers: HashMap<String, PeerInfo>,

    // File Management
    download_path: std::path::PathBuf,
    local_files: Vec<String>,
}

impl MyApp {
    pub fn new(tx: mpsc::Sender<AppCommand>, rx: mpsc::Receiver<AppEvent>) -> Self {
        let mut app = Self {
            cmd_sender: tx,
            event_receiver: rx,
            ui_state: AppUIState::default(),
            verification_state: VerificationState::default(),
            status_log: Vec::new(),
            peers: HashMap::new(),
            download_path: p2p_core::config::AppConfig::load().download_path,
            local_files: Vec::new(),
        };
        app.refresh_local_files();
        app
    }
    pub fn refresh_local_files(&mut self) {
        self.local_files.clear();
        if let Ok(entries) = std::fs::read_dir(&self.download_path) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    if meta.is_file() {
                        if let Some(name) = entry.file_name().to_str() {
                            self.local_files.push(name.to_string());
                        }
                    }
                }
            }
        }
        self.local_files.sort();
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
                AppEvent::PeerFound {
                    peer_id: _,
                    ip,
                    hostname,
                } => {
                    // Update or insert peer (using IP as key)
                    self.peers.insert(
                        ip.clone(),
                        PeerInfo {
                            ip,
                            hostname,
                            last_seen: Instant::now(),
                        },
                    );
                }
                AppEvent::ShowVerificationCode {
                    code,
                    from_ip,
                    from_name,
                } => {
                    self.verification_state = VerificationState::ShowingCode {
                        code,
                        from_ip,
                        from_name,
                    };
                }
                AppEvent::RequestVerificationCode { target_ip } => {
                    self.verification_state = VerificationState::InputtingCode {
                        target_ip,
                        code_input: String::new(),
                        error_msg: None,
                    };
                }
                AppEvent::PairingResult {
                    success,
                    peer_name,
                    message,
                } => {
                    self.status_log
                        .push(format!("Pairing with {}: {}", peer_name, message));

                    if !success {
                        if let VerificationState::InputtingCode { error_msg, .. } =
                            &mut self.verification_state
                        {
                            *error_msg = Some(message);
                            continue; // Keep window open to show error
                        }
                    }
                    // Close verification window on success or if redundant
                    self.verification_state = VerificationState::None;
                }
                AppEvent::TransferProgress {
                    file_name,
                    progress,
                    speed,
                } => {
                    self.status_log.push(format!(
                        "[Transfer] {} - {:.1}% @ {}",
                        file_name, progress, speed
                    ));
                }
                AppEvent::TransferCompleted(file_name) => {
                    self.status_log
                        .push(format!("[Transfer Complete] {}", file_name));
                    self.refresh_local_files();
                }
                AppEvent::Error(msg) => {
                    self.status_log.push(format!("[ERROR] {}", msg));
                }
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

            // Show status logs
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(100.0)
                .show(ui, |ui| {
                    for log in &self.status_log {
                        ui.label(log);
                    }
                });
        });

        // 5. Draw Floating Windows
        if self.ui_state.show_devices {
            ui::windows::devices::show(
                ctx,
                &mut self.ui_state.show_devices,
                &peer_list,
                &self.cmd_sender,
            );
        }

        if self.ui_state.show_files {
            // Clone for closure if needed, but here we can pass mutable references directly
            // temporary closure to handle refresh
            // We need to mutate self.local_files, so we can't borrow self immutably for 'files' AND mutably for 'refresh' callback easily if specific structure isn't split.
            // Simplification: trigger refresh flag.

            // Refactor: We can't pass a closure that borrows 'self' while 'self' is already borrowed.
            // We will handle refresh logic *after* the UI call if a flag is returned, or imply it from state change.
            // For now, let's just re-scan if needed.

            // Actually, simplest way in immediate mode:
            // Files panel takes &mut PathBuf and &Vec<String>.
            // If path changes, we detect it here.

            let current_path = self.download_path.clone(); // Clone to detect change
            let mut trigger_refresh = false;

            ui::windows::files::show(
                ctx,
                &mut self.ui_state.show_files,
                &mut self.download_path,
                &self.local_files,
                || {
                    trigger_refresh = true;
                },
            );

            if self.download_path != current_path || trigger_refresh {
                if self.download_path != current_path {
                    // Save config if path changed
                    let mut config = p2p_core::config::AppConfig::load();
                    config.download_path = self.download_path.clone();
                    config.save();
                }
                self.refresh_local_files();
            }
        }

        // 6. Draw Verification Windows
        verify::show_verification_windows(ctx, &mut self.verification_state, &self.cmd_sender);

        // Request repaint periodically to poll for new events from backend
        // This ensures we receive PeerFound events even when the peer list is empty
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}
