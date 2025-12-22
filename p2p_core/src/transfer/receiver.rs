use crate::{AppEvent, FileInfo};
use anyhow::Result;
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::constants::BUFFER_SIZE;
use super::hash::compute_file_hash;

/// Receive a single file from the stream
pub async fn receive_file(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    download_dir: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
    file_info: FileInfo,
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Receiving: {} ({} bytes)",
            file_info.file_name, file_info.file_size
        )))
        .await;

    // 1. Create download directory if needed
    tokio::fs::create_dir_all(download_dir).await?;
    let file_path = download_dir.join(&file_info.file_name);

    // 2. Check for partial file to resume
    let mut offset = 0;
    if file_path.exists() {
        let metadata = tokio::fs::metadata(&file_path).await?;
        let current_size = metadata.len();
        if current_size < file_info.file_size {
            offset = current_size;
        } else {
            // File already present and size >= remote - overwrite for safety
            offset = 0;
        }
    }

    // 3. Send Resume Info
    use super::protocol::{TransferMsg, send_msg};
    send_msg(send, &TransferMsg::ResumeInfo { offset }).await?;

    // 4. Open file
    let mut file = if offset > 0 {
        tokio::fs::OpenOptions::new()
            .write(true)
            .append(true)
            .open(&file_path)
            .await?
    } else {
        File::create(&file_path).await?
    };

    // 5. Receive file data
    let mut received: u64 = offset;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let total = file_info.file_size;
    let start_time = std::time::Instant::now();
    let mut last_progress_update = 0u64;

    // Send initial progress immediately so UI shows the transfer
    let initial_progress = (received as f32 / total as f32) * 100.0;
    let _ = event_tx
        .send(AppEvent::TransferProgress {
            file_name: file_info.file_name.clone(),
            progress: initial_progress,
            speed: "Starting...".to_string(),
            is_sending: false,
        })
        .await;

    while received < total {
        let to_read = std::cmp::min(BUFFER_SIZE as u64, total - received) as usize;
        let n = recv.read(&mut buffer[..to_read]).await?.unwrap_or(0);
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).await?;
        received += n as u64;

        // Report progress more frequently (every BUFFER_SIZE = 1MB or when complete)
        if received == total || received - last_progress_update >= BUFFER_SIZE as u64 {
            last_progress_update = received;
            let progress = (received as f32 / total as f32) * 100.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_bps = if elapsed > 0.0 {
                received as f64 / elapsed
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
                    file_name: file_info.file_name.clone(),
                    progress,
                    speed,
                    is_sending: false,
                })
                .await;
        }
    }

    file.flush().await?;

    // Verify file hash if provided
    if let Some(expected_hash) = file_info.file_hash {
        let _ = event_tx
            .send(AppEvent::VerificationStarted {
                file_name: file_info.file_name.clone(),
                is_sending: false,
            })
            .await;

        // Compute hash of received file
        let computed_hash = compute_file_hash(&file_path).await?;

        let verified = computed_hash == expected_hash;

        if !verified {
            let _ = event_tx
                .send(AppEvent::Error(format!(
                    "Hash verification FAILED for {}!",
                    file_info.file_name
                )))
                .await;
        }

        let _ = event_tx
            .send(AppEvent::VerificationCompleted {
                file_name: file_info.file_name.clone(),
                is_sending: false,
                verified,
            })
            .await;
    }

    let _ = event_tx
        .send(AppEvent::TransferCompleted(file_info.file_name.clone()))
        .await;

    Ok(())
}
