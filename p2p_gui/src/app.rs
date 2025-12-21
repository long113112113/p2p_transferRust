use crate::ui;
use crate::ui::windows::verify::{self, VerificationState};
use eframe::egui;
use p2p_core::{AppCommand, AppEvent};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Timeout for peer discovery - peers not seen within this time are pruned
const PEER_TIMEOUT_SECS: u64 = 12;

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

#[derive(Debug, Clone, Copy)]
enum VerificationStatus {
    Verifying,
    Verified,
    Failed,
}

struct TransferState {
    file_name: String,
    progress: f32,
    speed: String,
    is_sending: bool,
    verification_status: Option<VerificationStatus>,
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
    active_transfers: HashMap<String, TransferState>,
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
            download_path: p2p_core::config::get_download_dir(),
            local_files: Vec::new(),
            active_transfers: HashMap::new(),
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
                    self.status_log.push(format!("{}", msg));
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
                    is_sending,
                } => {
                    self.active_transfers.insert(
                        file_name.clone(),
                        TransferState {
                            file_name: file_name.clone(),
                            progress,
                            speed: speed.clone(),
                            is_sending,
                            verification_status: None,
                        },
                    );
                    // Progress already shown in progress bar, no need to log
                }
                AppEvent::TransferCompleted(file_name) => {
                    self.status_log
                        .push(format!("[Transfer Complete] {}", file_name));
                    self.active_transfers.remove(&file_name);
                    self.refresh_local_files();
                }
                AppEvent::Error(msg) => {
                    self.status_log.push(format!("[ERROR] {}", msg));
                }
                AppEvent::VerificationStarted {
                    file_name,
                    is_sending: _,
                } => {
                    if let Some(transfer) = self.active_transfers.get_mut(&file_name) {
                        transfer.verification_status = Some(VerificationStatus::Verifying);
                    }
                }
                AppEvent::VerificationCompleted {
                    file_name,
                    is_sending: _,
                    verified,
                } => {
                    if let Some(transfer) = self.active_transfers.get_mut(&file_name) {
                        transfer.verification_status = Some(if verified {
                            VerificationStatus::Verified
                        } else {
                            VerificationStatus::Failed
                        });
                    }
                    let status = if verified {
                        "✓ Verified"
                    } else {
                        "✗ Corrupted"
                    };
                    self.status_log
                        .push(format!("[Verification] {} - {}", file_name, status));
                }
            }
        }

        // 2. Prune offline peers - peers not seen within timeout are removed
        let now = Instant::now();
        self.peers.retain(|_, info| {
            now.duration_since(info.last_seen) < Duration::from_secs(PEER_TIMEOUT_SECS)
        });

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

            ui.separator();
            ui.heading("Active Transfers");
            if self.active_transfers.is_empty() {
                ui.label("No active transfers.");
            } else {
                for transfer in self.active_transfers.values() {
                    ui.group(|ui| {
                        let direction = if transfer.is_sending {
                            "Sending"
                        } else {
                            "Receiving"
                        };

                        // Show verification status if available
                        let verification_text = match transfer.verification_status {
                            Some(VerificationStatus::Verifying) => " ⏳ Verifying...",
                            Some(VerificationStatus::Verified) => " ✓ Verified",
                            Some(VerificationStatus::Failed) => " ✗ Corrupted",
                            None => "",
                        };

                        let label_text = format!(
                            "{} {}: {}{}",
                            direction, transfer.file_name, transfer.speed, verification_text
                        );

                        // Color code based on verification status
                        match transfer.verification_status {
                            Some(VerificationStatus::Verified) => {
                                ui.colored_label(egui::Color32::GREEN, label_text);
                            }
                            Some(VerificationStatus::Failed) => {
                                ui.colored_label(egui::Color32::RED, label_text);
                            }
                            _ => {
                                ui.label(label_text);
                            }
                        }

                        ui.add(egui::ProgressBar::new(transfer.progress / 100.0).show_percentage());
                    });
                }
            }

            // Show status logs
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(200.0) // Increased height
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
            let mut trigger_refresh = false;

            ui::windows::files::show(
                ctx,
                &mut self.ui_state.show_files,
                &self.download_path,
                &self.local_files,
                || {
                    trigger_refresh = true;
                },
            );

            if trigger_refresh {
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
