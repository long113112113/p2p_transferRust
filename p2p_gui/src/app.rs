use crate::ui;
use crate::ui::windows::qr_code::QrCodeCache;
use crate::ui::windows::upload_confirm::{self, UploadConfirmState};
use crate::ui::windows::verify::{self, VerificationState};
use crate::ui::windows::wan_connect::{self, WanConnectState};
use eframe::egui;
use p2p_core::{AppCommand, AppEvent};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};
use tokio::sync::mpsc;

/// Timeout for peer discovery - peers not seen within this time are pruned
const PEER_TIMEOUT_SECS: u64 = 12;

#[derive(Default)]
pub struct AppUIState {
    pub show_devices: bool,
    pub show_files: bool,
    pub show_qrcode: bool,
    pub show_wan_connect: bool,
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
    speed_bps: f64,
    is_sending: bool,
    verification_status: Option<VerificationStatus>,
}

/// Log entry with type for color coding
#[derive(Clone)]
enum LogType {
    Info,    // Default - gray/white
    Success, // Green
    Error,   // Red
    Warning, // Yellow/Orange
}

#[derive(Clone)]
struct LogEntry {
    message: String,
    log_type: LogType,
}

pub struct MyApp {
    // Channels
    cmd_sender: mpsc::Sender<AppCommand>,
    event_receiver: mpsc::Receiver<AppEvent>,
    event_sender: mpsc::Sender<AppEvent>,

    // App State
    ui_state: AppUIState,
    verification_state: VerificationState,
    upload_confirm_state: UploadConfirmState,

    // Data
    status_log: Vec<LogEntry>,
    // Key: IP address (unique identifier for now)
    peers: HashMap<String, PeerInfo>,

    // File Management
    download_path: std::path::PathBuf,
    local_files: Vec<String>,
    active_transfers: HashMap<String, TransferState>,

    // System Metrics
    system: System,
    last_metrics_update: Instant,

    // QR Code & HTTP Share
    qrcode_cache: QrCodeCache,
    share_url: String,
    http_server_running: bool,
    http_server_pending: bool,

    // WAN Connect
    wan_connect_state: WanConnectState,
}

impl MyApp {
    pub fn new(
        tx: mpsc::Sender<AppCommand>,
        rx: mpsc::Receiver<AppEvent>,
        event_tx: mpsc::Sender<AppEvent>,
    ) -> Self {
        let mut app = Self {
            cmd_sender: tx,
            event_receiver: rx,
            event_sender: event_tx,
            ui_state: AppUIState::default(),
            verification_state: VerificationState::default(),
            upload_confirm_state: UploadConfirmState::default(),
            status_log: Vec::new(),
            peers: HashMap::new(),
            download_path: p2p_core::config::get_download_dir(),
            local_files: Vec::new(),
            active_transfers: HashMap::new(),
            system: System::new_with_specifics(
                RefreshKind::nothing()
                    .with_cpu(CpuRefreshKind::everything())
                    .with_memory(MemoryRefreshKind::everything()),
            ),
            last_metrics_update: Instant::now(),
            qrcode_cache: QrCodeCache::default(),
            share_url: "Server not started".to_string(),
            http_server_running: false,
            http_server_pending: false,
            wan_connect_state: WanConnectState::default(),
        };
        app.refresh_local_files();
        app
    }

    pub fn refresh_local_files(&mut self) {
        self.local_files.clear();
        if let Ok(entries) = std::fs::read_dir(&self.download_path) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata()
                    && meta.is_file()
                    && let Some(name) = entry.file_name().to_str()
                {
                    self.local_files.push(name.to_string());
                }
            }
        }
        self.local_files.sort();
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.event_receiver.try_recv() {
            match event {
                AppEvent::Status(msg) => {
                    let log_type = if msg.contains("error")
                        || msg.contains("Error")
                        || msg.contains("ERROR")
                    {
                        LogType::Error
                    } else if msg.contains("Complete")
                        || msg.contains("success")
                        || msg.contains("Verified")
                    {
                        LogType::Success
                    } else if msg.contains("Connecting") || msg.contains("Starting") {
                        LogType::Warning
                    } else {
                        LogType::Info
                    };
                    self.status_log.push(LogEntry {
                        message: msg,
                        log_type,
                    });
                }
                AppEvent::PeerFound {
                    endpoint_id: _,
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
                    self.status_log.push(LogEntry {
                        message: format!("Pairing with {}: {}", peer_name, message),
                        log_type: if success {
                            LogType::Success
                        } else {
                            LogType::Error
                        },
                    });

                    if !success
                        && let VerificationState::InputtingCode { error_msg, .. } =
                            &mut self.verification_state
                    {
                        *error_msg = Some(message);
                        continue; // Keep window open to show error
                    }
                    self.verification_state = VerificationState::None;
                }

                AppEvent::TransferProgress {
                    file_name,
                    progress,
                    speed,
                    speed_bps,
                    is_sending,
                } => {
                    self.active_transfers
                        .entry(file_name.clone())
                        .and_modify(|t| {
                            t.progress = progress;
                            t.speed = speed.clone();
                            t.speed_bps = speed_bps;
                        })
                        .or_insert(TransferState {
                            file_name: file_name.clone(),
                            progress,
                            speed: speed.clone(),
                            speed_bps,
                            is_sending,
                            verification_status: None,
                        });
                }
                AppEvent::TransferCompleted(file_name) => {
                    self.status_log.push(LogEntry {
                        message: format!("Transfer Complete: {}", file_name),
                        log_type: LogType::Success,
                    });
                    self.active_transfers.remove(&file_name);
                    self.refresh_local_files();
                }
                AppEvent::Error(msg) => {
                    self.status_log.push(LogEntry {
                        message: format!("[ERROR] {}", msg),
                        log_type: LogType::Error,
                    });
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
                        format!("{} Verified", egui_phosphor::regular::CHECK_CIRCLE)
                    } else {
                        format!("{} Corrupted", egui_phosphor::regular::X_CIRCLE)
                    };
                    self.status_log.push(LogEntry {
                        message: format!("Verification: {} - {}", file_name, status),
                        log_type: if verified {
                            LogType::Success
                        } else {
                            LogType::Error
                        },
                    });
                }
                AppEvent::ShareUrlReady { url } => {
                    self.share_url = url;
                    // Reset QR cache to regenerate with new URL
                    self.qrcode_cache = QrCodeCache::default();
                }
                AppEvent::HttpServerStarted { url } => {
                    self.share_url = url;
                    self.http_server_running = true;
                    self.http_server_pending = false;
                    self.qrcode_cache = QrCodeCache::default();
                    self.status_log.push(LogEntry {
                        message: "HTTP server started".to_string(),
                        log_type: LogType::Success,
                    });
                }
                AppEvent::HttpServerStopped => {
                    self.http_server_running = false;
                    self.http_server_pending = false;
                    self.share_url = "Server not started".to_string();
                    self.status_log.push(LogEntry {
                        message: "HTTP server stopped".to_string(),
                        log_type: LogType::Info,
                    });
                }
                AppEvent::UploadRequest {
                    request_id,
                    file_name,
                    file_size,
                    from_ip,
                } => {
                    self.upload_confirm_state =
                        UploadConfirmState::Pending(upload_confirm::PendingUpload {
                            request_id,
                            file_name,
                            file_size,
                            from_ip,
                        });
                }
                AppEvent::UploadRequestCancelled { request_id } => {
                    if let UploadConfirmState::Pending(upload) = &self.upload_confirm_state {
                        if upload.request_id == request_id {
                            self.upload_confirm_state = UploadConfirmState::None;
                            self.status_log.push(LogEntry {
                                message: "Upload request cancelled".to_string(),
                                log_type: LogType::Info,
                            });
                        }
                    }
                }
                AppEvent::UploadProgress {
                    request_id: _,
                    received_bytes,
                    total_bytes: _,
                } => {
                    // Update main status log periodically or just use debug logs
                    // For now, let's log completion only to avoid spam,
                    // real-time progress is shown on the phone.
                    // Or we could show a special transfer in active_transfers list?
                    // Let's create a dummy transfer entry for visibility in GUI
                    // self.active_transfers.entry(request_id)...?
                    // Ideally we should track it by request_id but active_transfers uses file_name.
                    // Let's skip detailed progress in GUI for MVP upload, rely on "UploadCompleted".
                    // Or just log every 10%?
                    if received_bytes == 0 {
                        self.status_log.push(LogEntry {
                            message: format!("Incoming upload started..."),
                            log_type: LogType::Info,
                        });
                    }
                }
                AppEvent::UploadCompleted {
                    file_name,
                    saved_path: _,
                } => {
                    self.status_log.push(LogEntry {
                        message: format!("Upload received: {}", file_name),
                        log_type: LogType::Success,
                    });
                    self.refresh_local_files();
                }
            }
        }

        let now = Instant::now();
        self.peers.retain(|_, info| {
            now.duration_since(info.last_seen) < Duration::from_secs(PEER_TIMEOUT_SECS)
        });

        if now.duration_since(self.last_metrics_update) > Duration::from_secs(1) {
            self.system.refresh_cpu_all();
            self.system.refresh_memory();
            self.last_metrics_update = now;
        }

        // Calculate Bandwidth
        let mut total_upload = 0.0;
        let mut total_download = 0.0;

        for transfer in self.active_transfers.values() {
            let mbps = transfer.speed_bps / 1_000_000.0;
            if transfer.is_sending {
                total_upload += mbps;
            } else {
                total_download += mbps;
            }
        }

        let mut peer_list: Vec<String> = self
            .peers
            .values()
            .map(|info| format!("{} ({})", info.hostname, info.ip))
            .collect();
        peer_list.sort();

        ui::toolbar::show(ctx, &mut self.ui_state);
        egui::CentralPanel::default().show(ctx, |ui| {
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
                            Some(VerificationStatus::Verifying) => {
                                format!(" {} Verifying...", egui_phosphor::regular::TIMER)
                            }
                            Some(VerificationStatus::Verified) => {
                                format!(" {} Verified", egui_phosphor::regular::CHECK_CIRCLE)
                            }
                            Some(VerificationStatus::Failed) => {
                                format!(" {} Corrupted", egui_phosphor::regular::X_CIRCLE)
                            }
                            None => "".to_string(),
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

            // Show status logs with color coding
            ui.separator();
            ui.label("Status Logs:");
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for entry in &self.status_log {
                        let color = match entry.log_type {
                            LogType::Info => egui::Color32::GRAY,
                            LogType::Success => egui::Color32::from_rgb(100, 200, 100),
                            LogType::Error => egui::Color32::from_rgb(255, 100, 100),
                            LogType::Warning => egui::Color32::from_rgb(255, 200, 100),
                        };
                        ui.colored_label(color, &entry.message);
                    }
                });
        });

        // 5. Draw Bottom Status Bar (System Metrics)
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // CPU
                let cpu_usage = self.system.global_cpu_usage();
                ui.label(format!("CPU: {:.1}%", cpu_usage));
                ui.add(egui::ProgressBar::new(cpu_usage / 100.0).desired_width(100.0));

                ui.separator();

                // RAM
                let used_mem = self.system.used_memory() as f32 / 1024.0 / 1024.0 / 1024.0; // GB
                let total_mem = self.system.total_memory() as f32 / 1024.0 / 1024.0 / 1024.0; // GB
                let mem_ratio = if total_mem > 0.0 {
                    used_mem / total_mem
                } else {
                    0.0
                };
                ui.label(format!("RAM: {:.1}/{:.1} GB", used_mem, total_mem));
                ui.add(egui::ProgressBar::new(mem_ratio).desired_width(100.0));

                ui.separator();

                // Bandwidth
                ui.label(format!(
                    "{} Upload: {:.2} MB/s",
                    egui_phosphor::regular::UPLOAD_SIMPLE,
                    total_upload
                ));
                ui.label(format!(
                    "{} Download: {:.2} MB/s",
                    egui_phosphor::regular::DOWNLOAD_SIMPLE,
                    total_download
                ));
            });
        });

        // 6. Draw Floating Windows
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

        // QR Code Window
        if self.ui_state.show_qrcode {
            ui::windows::qr_code::show(
                ctx,
                &mut self.ui_state.show_qrcode,
                &mut self.qrcode_cache,
                &self.share_url,
                self.http_server_running,
                &mut self.http_server_pending,
                &self.cmd_sender,
            );
        }

        // 7. Draw Verification Windows
        verify::show_verification_windows(ctx, &mut self.verification_state, &self.cmd_sender);

        // 8. Draw Upload Confirmation Windows
        upload_confirm::show_upload_confirm_window(
            ctx,
            &mut self.upload_confirm_state,
            &self.cmd_sender,
        );

        // 9. Draw WAN Connect Window
        if self.ui_state.show_wan_connect {
            wan_connect::show(
                ctx,
                &mut self.ui_state.show_wan_connect,
                &mut self.wan_connect_state,
                &self.cmd_sender,
                &self.event_sender,
            );
        }

        // Request repaint periodically to poll for new events from backend
        // This ensures we receive PeerFound events even when the peer list is empty
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}
