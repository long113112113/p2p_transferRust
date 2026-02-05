use anyhow::Result;
use p2p_core::{AppEvent, FileInfo};
use p2p_core::transfer::utils::{open_secure_file, validate_transfer_info};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tracing::info;

use crate::protocol::{WanTransferMsg, send_msg};

/// Buffer size for file transfer (16MB)
const BUFFER_SIZE: usize = 16 * 1024 * 1024;

/// Receive a single file from the stream
///
/// # Arguments
/// * `send` - Send stream for protocol messages
/// * `recv` - Receive stream for data
/// * `download_dir` - Directory to save received files
/// * `event_tx` - Channel to send progress events to GUI
/// * `file_info` - File metadata received from sender
pub async fn receive_file(
    send: &mut iroh::endpoint::SendStream,
    recv: &mut iroh::endpoint::RecvStream,
    download_dir: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
    mut file_info: FileInfo,
) -> Result<()> {
    // Security check: Validate file size and name length
    if let Err(e) = validate_transfer_info(&file_info.file_name, file_info.file_size) {
        let err_msg = e.to_string();
        tracing::error!("File validation failed: {}", err_msg);
        let _ = send_msg(
            send,
            &WanTransferMsg::Error {
                message: err_msg.clone(),
            },
        )
        .await;
        let _ = event_tx.send(AppEvent::Error(err_msg.clone())).await;
        return Err(e);
    }

    let file_name = sanitize_file_name(&file_info.file_name);
    // Update file_info name with sanitized version
    file_info.file_name = file_name.clone();
    let file_size = file_info.file_size;

    info!("Receiving file: {} ({} bytes)", file_name, file_size);
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Receiving: {} ({} bytes)",
            file_name, file_size
        )))
        .await;

    tokio::fs::create_dir_all(download_dir).await?;
    let file_path = download_dir.join(&file_name);
    let mut offset = 0u64;
    if file_path.exists() {
        let metadata = tokio::fs::metadata(&file_path).await?;
        let current_size = metadata.len();
        if current_size < file_size {
            offset = current_size;
            info!("Resuming from offset: {}", offset);
        } else if current_size == file_size {
            info!("File already complete, skipping transfer");
            send_msg(send, &WanTransferMsg::ResumeInfo { offset: file_size }).await?;
            send_msg(send, &WanTransferMsg::TransferComplete).await?;
            let _ = event_tx
                .send(AppEvent::TransferCompleted(file_name.clone()))
                .await;
            return Ok(());
        } else {
            info!("File size mismatch, restarting transfer");
            tokio::fs::remove_file(&file_path).await?;
            offset = 0;
        }
    }

    send_msg(send, &WanTransferMsg::ResumeInfo { offset }).await?;

    // Use open_secure_file to ensure secure permissions (0o600 on Unix)
    let mut file = open_secure_file(&file_path, offset).await?;

    let mut received: u64 = offset;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let start_time = std::time::Instant::now();
    let mut last_progress_update = 0u64;

    report_progress(
        event_tx, &file_name, received, file_size, start_time, offset, false,
    )
    .await;

    while received < file_size {
        let to_read = std::cmp::min(BUFFER_SIZE as u64, file_size - received) as usize;
        match recv.read(&mut buffer[..to_read]).await {
            Ok(Some(n)) => {
                if n == 0 {
                    if received < file_size {
                        let err_msg = format!(
                            "Stream closed early: received {}/{} bytes",
                            received, file_size
                        );
                        tracing::error!("{}", err_msg);
                        send_msg(
                            send,
                            &WanTransferMsg::Error {
                                message: err_msg.clone(),
                            },
                        )
                        .await?;
                        return Err(anyhow::anyhow!(err_msg));
                    }
                    break;
                }

                file.write_all(&buffer[..n]).await?;
                received += n as u64;

                if received == file_size || received - last_progress_update >= BUFFER_SIZE as u64 {
                    last_progress_update = received;
                    report_progress(
                        event_tx, &file_name, received, file_size, start_time, offset, false,
                    )
                    .await;
                }
            }
            Ok(None) => {
                if received < file_size {
                    let err_msg = format!(
                        "Stream closed early: received {}/{} bytes",
                        received, file_size
                    );
                    tracing::error!("{}", err_msg);
                    send_msg(
                        send,
                        &WanTransferMsg::Error {
                            message: err_msg.clone(),
                        },
                    )
                    .await?;
                    return Err(anyhow::anyhow!(err_msg));
                }
                break;
            }
            Err(e) => {
                let err_msg = format!("Read error: {}", e);
                tracing::error!("{}", err_msg);
                send_msg(
                    send,
                    &WanTransferMsg::Error {
                        message: err_msg.clone(),
                    },
                )
                .await?;
                return Err(anyhow::anyhow!(err_msg));
            }
        }
    }

    file.flush().await?;

    if received != file_size {
        let err_msg = format!(
            "Incomplete transfer: received {}/{} bytes",
            received, file_size
        );
        tracing::error!("{}", err_msg);
        send_msg(
            send,
            &WanTransferMsg::Error {
                message: err_msg.clone(),
            },
        )
        .await?;
        return Err(anyhow::anyhow!(err_msg));
    }

    info!("File received successfully: {}", file_name);

    if let Some(expected_hash) = file_info.file_hash {
        let _ = event_tx
            .send(AppEvent::VerificationStarted {
                file_name: file_name.clone(),
                is_sending: false,
            })
            .await;

        let computed_hash = p2p_core::transfer::hash::compute_file_hash(&file_path).await?;
        let verified = computed_hash == expected_hash;

        if !verified {
            tracing::error!(
                "Hash verification FAILED for {}: expected {} got {}",
                file_name,
                &expected_hash[..16],
                &computed_hash[..16]
            );
            let _ = event_tx
                .send(AppEvent::Error(format!(
                    "Hash verification FAILED for {}!",
                    file_name
                )))
                .await;
        } else {
            info!("Hash verification passed for {}", file_name);
        }

        let _ = event_tx
            .send(AppEvent::VerificationCompleted {
                file_name: file_name.clone(),
                is_sending: false,
                verified,
            })
            .await;
    }

    send_msg(send, &WanTransferMsg::TransferComplete).await?;

    let _ = event_tx
        .send(AppEvent::TransferCompleted(file_name.clone()))
        .await;

    Ok(())
}

/// Sanitize file name to prevent path traversal attacks
fn sanitize_file_name(file_name: &str) -> String {
    Path::new(file_name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown_file")
        .to_string()
}

/// Report transfer progress to the event channel
async fn report_progress(
    event_tx: &mpsc::Sender<AppEvent>,
    file_name: &str,
    bytes_done: u64,
    total_bytes: u64,
    start_time: std::time::Instant,
    offset: u64,
    is_sending: bool,
) {
    let progress = (bytes_done as f32 / total_bytes as f32) * 100.0;
    let elapsed = start_time.elapsed().as_secs_f64();
    let speed_bps = if elapsed > 0.0 {
        bytes_done.saturating_sub(offset) as f64 / elapsed
    } else {
        0.0
    };
    let speed = format_transfer_speed(bytes_done.saturating_sub(offset), elapsed);

    let _ = event_tx
        .send(AppEvent::TransferProgress {
            file_name: file_name.to_string(),
            progress,
            speed,
            speed_bps,
            is_sending,
        })
        .await;
}

/// Format transfer speed from bytes and elapsed time
fn format_transfer_speed(bytes_transferred: u64, elapsed_secs: f64) -> String {
    if elapsed_secs <= 0.0 {
        return "Starting...".to_string();
    }

    let speed_bps = bytes_transferred as f64 / elapsed_secs;
    if speed_bps > 1_000_000.0 {
        format!("{:.2} MB/s", speed_bps / 1_000_000.0)
    } else if speed_bps > 1_000.0 {
        format!("{:.1} KB/s", speed_bps / 1_000.0)
    } else {
        format!("{:.0} B/s", speed_bps)
    }
}
