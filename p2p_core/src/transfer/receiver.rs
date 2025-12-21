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
    stream: &mut quinn::RecvStream,
    download_dir: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] receive_file: Starting to read metadata length...".to_string(),
        ))
        .await;

    // 1. Read metadata length (4 bytes, big-endian)
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let meta_len = u32::from_be_bytes(len_buf) as usize;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Metadata length = {} bytes",
            meta_len
        )))
        .await;

    // 2. Read metadata JSON
    let mut meta_buf = vec![0u8; meta_len];
    stream.read_exact(&mut meta_buf).await?;
    let file_info: FileInfo = serde_json::from_slice(&meta_buf)?;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: FileInfo parsed - name: {}, size: {}",
            file_info.file_name, file_info.file_size
        )))
        .await;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Receiving: {} ({} bytes)",
            file_info.file_name, file_info.file_size
        )))
        .await;

    // 3. Create download directory if needed
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Creating dir {:?}",
            download_dir
        )))
        .await;
    tokio::fs::create_dir_all(download_dir).await?;

    // 4. Create file
    let file_path = download_dir.join(&file_info.file_name);
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Creating file {:?}",
            file_path
        )))
        .await;
    let mut file = File::create(&file_path).await?;
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] receive_file: File created successfully".to_string(),
        ))
        .await;

    // 5. Receive file data
    let mut received: u64 = 0;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let total = file_info.file_size;
    let start_time = std::time::Instant::now();

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Starting to receive {} bytes...",
            total
        )))
        .await;

    while received < total {
        let to_read = std::cmp::min(BUFFER_SIZE as u64, total - received) as usize;
        let n = stream.read(&mut buffer[..to_read]).await?.unwrap_or(0);
        if n == 0 {
            let _ = event_tx
                .send(AppEvent::Status(format!(
                    "[DEBUG] receive_file: Stream returned 0 bytes. Received {}/{} bytes",
                    received, total
                )))
                .await;
            break;
        }
        file.write_all(&buffer[..n]).await?;
        received += n as u64;

        // Report progress (less frequent to avoid log spam)
        if received == total || received % (BUFFER_SIZE as u64 * 10) == 0 {
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

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Loop finished. Received {}/{} bytes",
            received, total
        )))
        .await;

    file.flush().await?;

    // Verify file hash if provided
    if let Some(expected_hash) = file_info.file_hash {
        let _ = event_tx
            .send(AppEvent::VerificationStarted {
                file_name: file_info.file_name.clone(),
                is_sending: false,
            })
            .await;

        let _ = event_tx
            .send(AppEvent::Status(format!(
                "[DEBUG] Verifying hash for {}...",
                file_info.file_name
            )))
            .await;

        // Compute hash of received file
        let computed_hash = compute_file_hash(&file_path).await?;

        let verified = computed_hash == expected_hash;

        if verified {
            let _ = event_tx
                .send(AppEvent::Status(format!(
                    "[DEBUG] Hash verification SUCCESS for {}",
                    file_info.file_name
                )))
                .await;
        } else {
            let _ = event_tx
                .send(AppEvent::Error(format!(
                    "Hash verification FAILED for {}! Expected: {}..., Got: {}...",
                    file_info.file_name,
                    &expected_hash[..16],
                    &computed_hash[..16]
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
    } else {
        let _ = event_tx
            .send(AppEvent::Status(format!(
                "[DEBUG] No hash provided for {}, skipping verification",
                file_info.file_name
            )))
            .await;
    }

    let _ = event_tx
        .send(AppEvent::TransferCompleted(file_info.file_name.clone()))
        .await;

    Ok(())
}
