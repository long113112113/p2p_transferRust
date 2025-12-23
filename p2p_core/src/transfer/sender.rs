use crate::{AppEvent, FileInfo};
use anyhow::{Result, anyhow};
use quinn::Endpoint;
use std::net::SocketAddr;
use std::path::PathBuf;

use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;

use super::constants::BUFFER_SIZE;
use super::hash::compute_file_hash;
use super::protocol::{TransferMsg, recv_msg, send_msg};
use super::utils::report_progress;

/// Context for file transfers containing peer information
#[derive(Debug, Clone)]
pub struct TransferContext {
    pub my_peer_id: String,
    pub my_name: String,
    pub target_peer_name: String,
}

/// Send files to a remote peer
pub async fn send_files(
    endpoint: &Endpoint,
    target_addr: SocketAddr,
    files: Vec<PathBuf>,
    event_tx: mpsc::Sender<AppEvent>,
    context: TransferContext,
    input_code_rx: Option<tokio::sync::oneshot::Receiver<String>>,
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Connecting to: {} ({})",
            context.target_peer_name, target_addr
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
        context.clone(),
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

    let mut handles = Vec::new();

    for file_path in files.iter() {
        let connection = connection.clone();
        let file_path = file_path.clone();
        let event_tx = event_tx.clone();

        let handle = tokio::spawn(async move {
            if let Err(e) = send_single_file(&connection, &file_path, &event_tx).await {
                let _ = event_tx
                    .send(AppEvent::Error(format!(
                        "Error sending {}: {}",
                        file_path.display(),
                        e
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
    context: TransferContext,
    target_addr: SocketAddr,
    input_code_rx: Option<tokio::sync::oneshot::Receiver<String>>,
) -> Result<()> {
    send_msg(
        send,
        &TransferMsg::PairingRequest {
            peer_id: context.my_peer_id.clone(),
            peer_name: context.my_name.clone(),
        },
    )
    .await?;

    let msg = recv_msg(recv).await?;
    match msg {
        TransferMsg::PairingAccepted => {
            let _ = event_tx
                .send(AppEvent::PairingResult {
                    success: true,
                    peer_name: context.target_peer_name.clone(),
                    message: "Already paired.".to_string(),
                })
                .await;
            Ok(())
        }
        TransferMsg::VerificationRequired => {
            let _ = event_tx
                .send(AppEvent::RequestVerificationCode {
                    target_ip: target_addr.ip().to_string(),
                })
                .await;

            let _ = event_tx
                .send(AppEvent::Status(
                    "Please enter the verification code shown on the other device...".to_string(),
                ))
                .await;

            let code = if let Some(rx) = input_code_rx {
                match rx.await {
                    Ok(c) => c,
                    Err(_) => return Err(anyhow!("User cancelled verification input")),
                }
            } else {
                return Err(anyhow!("No input channel provided for verification code"));
            };

            send_msg(send, &TransferMsg::VerificationCode { code }).await?;

            let result_msg = recv_msg(recv).await?;
            match result_msg {
                TransferMsg::VerificationSuccess => {
                    let _ = event_tx
                        .send(AppEvent::PairingResult {
                            success: true,
                            peer_name: context.target_peer_name,
                            message: "Verification successful".to_string(),
                        })
                        .await;
                    Ok(())
                }
                TransferMsg::VerificationFailed { message } => {
                    let _ = event_tx
                        .send(AppEvent::PairingResult {
                            success: false,
                            peer_name: context.target_peer_name,
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
    let file_hash = compute_file_hash(file_path).await?;

    let (mut send_stream, mut recv_stream) = connection.open_bi().await?;

    let file_info = FileInfo {
        file_name: file_name.clone(),
        file_size,
        file_path: PathBuf::new(),
        file_hash: Some(file_hash.clone()),
    };

    send_msg(
        &mut send_stream,
        &TransferMsg::FileMetadata { info: file_info },
    )
    .await?;

    let msg = recv_msg(&mut recv_stream).await?;
    let offset = match msg {
        TransferMsg::ResumeInfo { offset } => offset,
        _ => return Err(anyhow!("Expected ResumeInfo, got {:?}", msg)),
    };

    if offset > 0 {
        use std::io::SeekFrom;
        file.seek(SeekFrom::Start(offset)).await?;
    }

    let mut sent: u64 = offset;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let start_time = std::time::Instant::now();
    let mut last_progress_update = 0u64;

    report_progress(
        event_tx, &file_name, sent, file_size, start_time, offset, true,
    )
    .await;

    loop {
        //Read file to buffer
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        //Send buffer to remote peer
        send_stream.write_all(&buffer[..n]).await?;
        sent += n as u64;

        // Report progress more frequently (every BUFFER_SIZE = 1MB or when complete)
        if sent == file_size || sent - last_progress_update >= BUFFER_SIZE as u64 {
            last_progress_update = sent;
            report_progress(
                event_tx, &file_name, sent, file_size, start_time, offset, true,
            )
            .await;
        }
    }

    // Finish stream
    send_stream.finish()?;

    // Wait for receiver to confirm completion (they send this after flushing and verifying)
    // We expect TransferComplete. If we close too early, receiver gets "Connection Lost".
    match recv_msg(&mut recv_stream).await {
        Ok(TransferMsg::TransferComplete) => {
            // Transfer confirmed by receiver
        }
        Ok(msg) => {
            let _ = event_tx
                .send(AppEvent::Error(format!(
                    "Unexpected completion message: {:?}",
                    msg
                )))
                .await;
        }
        Err(e) => {
            // If receiving failed, it might mean connection dropped or peer closed, which is bad at this stage
            let _ = event_tx
                .send(AppEvent::Error(format!(
                    "Failed to receive completion ack: {}",
                    e
                )))
                .await;
        }
    }

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
