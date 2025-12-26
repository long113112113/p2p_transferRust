use eframe::egui;
use p2p_core::AppCommand;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct PendingUpload {
    pub request_id: String,
    pub file_name: String,
    pub file_size: u64,
    pub from_ip: String,
}

#[derive(Debug, Clone, Default)]
pub enum UploadConfirmState {
    #[default]
    None,
    /// Pending upload request waiting for user approval
    Pending(PendingUpload),
}

/// Render upload confirmation window
pub fn show_upload_confirm_window(
    ctx: &egui::Context,
    state: &mut UploadConfirmState,
    cmd_tx: &mpsc::Sender<AppCommand>,
) {
    let mut open = true;
    let mut should_close = false;

    if let UploadConfirmState::Pending(upload) = state {
        egui::Window::new("File Upload Request")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!(
                    "Device ({}) wants to send you a file:",
                    upload.from_ip
                ));
                ui.add_space(10.0);

                ui.group(|ui| {
                    ui.label(format!("File: {}", upload.file_name));
                    ui.label(format!("Size: {}", format_size(upload.file_size)));
                });

                ui.add_space(15.0);

                ui.horizontal(|ui| {
                    if ui.button("Accept").clicked() {
                        let _ = cmd_tx.blocking_send(AppCommand::RespondUploadRequest {
                            request_id: upload.request_id.clone(),
                            accepted: true,
                        });
                        should_close = true;
                    }

                    if ui.button("Reject").clicked() {
                        let _ = cmd_tx.blocking_send(AppCommand::RespondUploadRequest {
                            request_id: upload.request_id.clone(),
                            accepted: false,
                        });
                        should_close = true;
                    }
                });
            });

        if !open || should_close {
            *state = UploadConfirmState::None;
        }
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
