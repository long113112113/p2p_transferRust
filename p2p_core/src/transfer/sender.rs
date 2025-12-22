use crate::{AppEvent, FileInfo};
use anyhow::{Result, anyhow};
use quinn::Endpoint;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;

use super::constants::BUFFER_SIZE;
use super::hash::compute_file_hash;
use super::protocol::{TransferMsg, recv_msg, send_msg};

/// Send files to a remote peer
pub async fn send_files(
    endpoint: &Endpoint,
    target_addr: SocketAddr,
    files: Vec<PathBuf>,
    event_tx: mpsc::Sender<AppEvent>,
    my_peer_id: String,
    my_name: String,
    target_peer_name: String,
    input_code_rx: Option<tokio::sync::oneshot::Receiver<String>>,
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] send_files called. Target: {}, Files: {:?}",
            target_addr, files
        )))
        .await;
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Connecting to: {} ({})",
            target_peer_name, target_addr
        )))
        .await;

    // Connect to peer
    let connection = endpoint.connect(target_addr, "localhost")?.await?;

    // Perform verification handshake
    let (mut send_stream, mut recv_stream) = connection.open_bi().await?;
    if let Err(e) = perform_verification_handshake(
        &mut send_stream,
        &mut recv_stream,
        &event_tx,
        my_peer_id,
        my_name,
        target_peer_name.clone(),
        target_addr,
        input_code_rx,
    )
    .await
    {
        return Err(anyhow!("Handshake failed: {}", e));
    }

    let _ = event_tx
        .send(AppEvent::Status(
            "Connected and verified. Starting file transfer...".to_string(),
        ))
        .await;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] Starting file transfer. Total files: {}",
            files.len()
        )))
        .await;

    let mut handles = Vec::new();

    for (idx, file_path) in files.iter().enumerate() {
        let connection = connection.clone();
        let file_path = file_path.clone();
        let event_tx = event_tx.clone();
        let idx = idx;
        let total_files = files.len();

        let handle = tokio::spawn(async move {
            let _ = event_tx
                .send(AppEvent::Status(format!(
                    "[DEBUG] Sending file {}/{}: {}",
                    idx + 1,
                    total_files,
                    file_path.display()
                )))
                .await;

            if let Err(e) = send_single_file(&connection, &file_path, &event_tx).await {
                let _ = event_tx
                    .send(AppEvent::Error(format!(
                        "Error sending {}: {}",
                        file_path.display(),
                        e
                    )))
                    .await;
            } else {
                let _ = event_tx
                    .send(AppEvent::Status(format!(
                        "[DEBUG] File sent successfully: {}",
                        file_path.display()
                    )))
                    .await;
            }
        });
        handles.push(handle);
    }

    // Wait for all transfers to complete
    for handle in handles {
        if let Err(e) = handle.await {
            let _ = event_tx
                .send(AppEvent::Error(format!("Task join error: {}", e)))
                .await;
        }
    }

    Ok(())
}

/// Perform verification handshake on sender side
async fn perform_verification_handshake(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    event_tx: &mpsc::Sender<AppEvent>,
    my_peer_id: String,
    my_name: String,
    target_peer_name: String,
    target_addr: SocketAddr,
    input_code_rx: Option<tokio::sync::oneshot::Receiver<String>>,
) -> Result<()> {
    // 1. Send PairingRequest
    send_msg(
        send,
        &TransferMsg::PairingRequest {
            peer_id: my_peer_id,
            peer_name: my_name,
        },
    )
    .await?;

    // 2. Wait for response
    let msg = recv_msg(recv).await?;
    match msg {
        TransferMsg::PairingAccepted => {
            // Already paired
            let _ = event_tx
                .send(AppEvent::PairingResult {
                    success: true,
                    peer_name: target_peer_name,
                    message: "Already paired.".to_string(),
                })
                .await;
            Ok(())
        }
        TransferMsg::VerificationRequired => {
            // Need verification
            let _ = event_tx
                .send(AppEvent::RequestVerificationCode {
                    target_ip: target_addr.ip().to_string(),
                })
                .await;

            let _ = event_tx
                .send(AppEvent::Status(
                    "Vui lòng nhập mã xác thực trên máy kia...".to_string(),
                ))
                .await;

            // Wait for user input from GUI via channel
            let code = if let Some(rx) = input_code_rx {
                match rx.await {
                    Ok(c) => c,
                    Err(_) => return Err(anyhow!("User cancelled verification input")),
                }
            } else {
                return Err(anyhow!("No input channel provided for verification code"));
            };

            send_msg(send, &TransferMsg::VerificationCode { code }).await?;

            // 3. Wait for final result
            let result_msg = recv_msg(recv).await?;
            match result_msg {
                TransferMsg::VerificationSuccess => {
                    let _ = event_tx
                        .send(AppEvent::PairingResult {
                            success: true,
                            peer_name: target_peer_name,
                            message: "Xác thực thành công".to_string(),
                        })
                        .await;
                    Ok(())
                }
                TransferMsg::VerificationFailed { message } => {
                    let _ = event_tx
                        .send(AppEvent::PairingResult {
                            success: false,
                            peer_name: target_peer_name,
                            message: message.clone(),
                        })
                        .await;
                    Err(anyhow!("Verification failed: {}", message))
                }
                _ => Err(anyhow!("Unexpected response: {:?}", result_msg)),
            }
        }
        _ => Err(anyhow!("Unexpected handshake response: {:?}", msg)),
    }
}

/// Send a single file through the connection
async fn send_single_file(
    connection: &quinn::Connection,
    file_path: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] send_single_file: Opening file {}",
            file_path.display()
        )))
        .await;

    // Open file
    let mut file = File::open(file_path).await?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Invalid file name"))?
        .to_string();

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Sending: {} ({} bytes)",
            file_name, file_size
        )))
        .await;

    // Compute hash before sending
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] Computing hash for {}...",
            file_name
        )))
        .await;

    let file_hash = compute_file_hash(file_path).await?;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] Hash computed: {}",
            &file_hash[..16] // Show first 16 chars
        )))
        .await;

    // Open bi-directional stream
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] Opening bi-directional stream...".to_string(),
        ))
        .await;
    let (mut send_stream, mut recv_stream) = connection.open_bi().await?;
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] Bi-directional stream opened.".to_string(),
        ))
        .await;

    // 1. Send metadata with hash
    let file_info = FileInfo {
        file_name: file_name.clone(),
        file_size,
        file_path: PathBuf::new(), // Not needed for transfer
        file_hash: Some(file_hash.clone()),
    };

    // Wrap in TransferMsg
    send_msg(
        &mut send_stream,
        &TransferMsg::FileMetadata { info: file_info },
    )
    .await?;

    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] Metadata sent successfully. Waiting for resume info...".to_string(),
        ))
        .await;

    // 2. Wait for Resume Info
    let msg = recv_msg(&mut recv_stream).await?;
    let offset = match msg {
        TransferMsg::ResumeInfo { offset } => offset,
        _ => return Err(anyhow!("Expected ResumeInfo, got {:?}", msg)),
    };

    if offset > 0 {
        let _ = event_tx
            .send(AppEvent::Status(format!(
                "[DEBUG] Resuming transfer from offset {}",
                offset
            )))
            .await;
        use std::io::SeekFrom;
        file.seek(SeekFrom::Start(offset)).await?;
    }

    // 3. Send file data
    let mut sent: u64 = offset;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let start_time = std::time::Instant::now();
    let mut last_progress_update = 0u64;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] Starting to send file data from offset {}: {} bytes remaining",
            offset,
            file_size - offset
        )))
        .await;

    // Send initial progress immediately so UI shows the transfer
    let initial_progress = (sent as f32 / file_size as f32) * 100.0;
    let _ = event_tx
        .send(AppEvent::TransferProgress {
            file_name: file_name.clone(),
            progress: initial_progress,
            speed: "Starting...".to_string(),
            is_sending: true,
        })
        .await;

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            let _ = event_tx
                .send(AppEvent::Status(format!(
                    "[DEBUG] File read complete. Total sent: {} bytes",
                    sent
                )))
                .await;
            break;
        }
        send_stream.write_all(&buffer[..n]).await?;
        sent += n as u64;

        // Report progress more frequently (every BUFFER_SIZE = 1MB or when complete)
        if sent == file_size || sent - last_progress_update >= BUFFER_SIZE as u64 {
            last_progress_update = sent;
            let progress = (sent as f32 / file_size as f32) * 100.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_bps = if elapsed > 0.0 {
                (sent - offset) as f64 / elapsed
            } else {
                0.0
            };
            let speed = if speed_bps > 1_000_000.0 {
                format!("{:.2} MB/s", speed_bps / 1_000_000.0)
            } else {
                format!("{:.1} KB/s", speed_bps / 1_000.0)
            };
            let _ = event_tx
                .send(AppEvent::TransferProgress {
                    file_name: file_name.clone(),
                    progress,
                    speed,
                    is_sending: true,
                })
                .await;
        }
    }

    // Finish stream
    let _ = event_tx
        .send(AppEvent::Status("[DEBUG] Finishing stream...".to_string()))
        .await;
    send_stream.finish()?;

    // Wait a short time for data to be flushed
    // We use a timeout instead of stopped().await because receiver might not send STOP_SENDING
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] Waiting for data to flush (max 2s)...".to_string(),
        ))
        .await;

    // Use tokio timeout to avoid blocking forever
    let _ = tokio::time::timeout(Duration::from_secs(2), send_stream.stopped()).await;

    let _ = event_tx
        .send(AppEvent::Status("[DEBUG] Stream finished.".to_string()))
        .await;

    // Notify sender that file was sent and verified (we assume receiver will verify)
    let _ = event_tx
        .send(AppEvent::VerificationCompleted {
            file_name: file_name.clone(),
            is_sending: true,
            verified: true, // Sender side always true (we computed the hash)
        })
        .await;

    let _ = event_tx.send(AppEvent::TransferCompleted(file_name)).await;

    Ok(())
}
