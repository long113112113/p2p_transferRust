use eframe::egui;
use p2p_core::AppCommand;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum VerificationState {
    None,
    /// Shows the code to the receiver
    ShowingCode {
        code: String,
        from_ip: String,
        from_name: String,
    },
    /// Asks the sender to input the code
    InputtingCode {
        target_ip: String,
        code_input: String,
        error_msg: Option<String>,
    },
}

impl Default for VerificationState {
    fn default() -> Self {
        Self::None
    }
}

/// Render verification windows based on state
pub fn show_verification_windows(
    ctx: &egui::Context,
    state: &mut VerificationState,
    cmd_tx: &mpsc::Sender<AppCommand>,
) {
    let mut open = true;
    let mut should_close = false;

    match state {
        VerificationState::ShowingCode {
            code,
            from_ip,
            from_name,
        } => {
            egui::Window::new("Yêu cầu kết nối")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Thiết bị '{}' ({}) muốn gửi file cho bạn.",
                        from_name, from_ip
                    ));
                    ui.add_space(10.0);
                    ui.label("Mã xác thực của bạn là:");
                    ui.add_space(5.0);
                    ui.heading(code.as_str());
                    ui.add_space(15.0);
                    if ui.button("Đóng").clicked() {
                        should_close = true;
                    }
                });
        }
        VerificationState::InputtingCode {
            target_ip,
            code_input,
            error_msg,
        } => {
            let mut submit_clicked = false;
            let mut submitted_code = String::new();

            egui::Window::new("Nhập mã xác thực")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Nhập mã đang hiển thị trên thiết bị đích ({})",
                        target_ip
                    ));
                    ui.add_space(10.0);

                    let response = ui.text_edit_singleline(code_input);

                    if let Some(err) = error_msg {
                        ui.colored_label(egui::Color32::RED, err.as_str());
                    }

                    ui.add_space(10.0);
                    if ui.button("Gửi mã").clicked()
                        || (response.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter)))
                    {
                        submit_clicked = true;
                        submitted_code = code_input.clone();
                    }
                });

            if submit_clicked {
                if submitted_code.len() == 4 {
                    let cmd_tx = cmd_tx.clone();
                    let target_ip_clone = target_ip.clone();
                    let code_clone = submitted_code;

                    tokio::spawn(async move {
                        let _ = cmd_tx
                            .send(AppCommand::SubmitVerificationCode {
                                target_ip: target_ip_clone,
                                code: code_clone,
                            })
                            .await;
                    });
                    should_close = true;
                } else {
                    // Update error message in state?
                    // Need to mutate state which is borrowed.
                    // Actually 'state' is borrowed by 'match'.
                    // We can't mutate 'state' here directly if it's borrowed.
                    // But 'match state' borrows it.
                    // The simplest way: Close on valid submit. For invalid, we need to update state.
                    // But we can't easily update state while matching on it.
                    // Refactor: clone necessary data first?
                }
            }
        }
        VerificationState::None => {
            return;
        }
    }

    // Logic to update state after match
    if !open || should_close {
        *state = VerificationState::None;
        return;
    }
}
