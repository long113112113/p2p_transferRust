use crate::{AppEvent, FileInfo};
use anyhow::Result;
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

use super::constants::BUFFER_SIZE;
use super::hash::compute_file_hash;
use super::utils::{report_progress, sanitize_file_name};

/// Receive a single file from the stream
pub async fn receive_file(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    download_dir: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
    mut file_info: FileInfo,
) -> Result<()> {
    file_info.file_name = sanitize_file_name(&file_info.file_name);
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Receiving: {} ({} bytes)",
            file_info.file_name, file_info.file_size
        )))
        .await;

    tokio::fs::create_dir_all(download_dir).await?;
    let file_path = download_dir.join(&file_info.file_name);

    let mut offset = 0;
    if file_path.exists() {
        let metadata = tokio::fs::metadata(&file_path).await?;
        let current_size = metadata.len();
        if current_size < file_info.file_size {
            offset = current_size;
        } else {
            offset = 0;
        }
    }

    use super::protocol::{TransferMsg, send_msg};
    send_msg(send, &TransferMsg::ResumeInfo { offset }).await?;

    let mut file = if offset > 0 {
        tokio::fs::OpenOptions::new()
            .write(true)
            .append(true)
            .open(&file_path)
            .await?
    } else {
        File::create(&file_path).await?
    };

    let mut received: u64 = offset;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let total = file_info.file_size;
    let start_time = std::time::Instant::now();
    let mut last_progress_update = 0u64;

    report_progress(
        event_tx,
        &file_info.file_name,
        received,
        total,
        start_time,
        offset,
        false,
    )
    .await;

    while received < total {
        let to_read = std::cmp::min(BUFFER_SIZE as u64, total - received) as usize;
        let n = recv.read(&mut buffer[..to_read]).await?.unwrap_or(0);
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).await?;
        received += n as u64;

        if received == total || received - last_progress_update >= BUFFER_SIZE as u64 {
            last_progress_update = received;
            report_progress(
                event_tx,
                &file_info.file_name,
                received,
                total,
                start_time,
                offset,
                false,
            )
            .await;
        }
    }

    file.flush().await?;

    if let Some(expected_hash) = file_info.file_hash {
        let _ = event_tx
            .send(AppEvent::VerificationStarted {
                file_name: file_info.file_name.clone(),
                is_sending: false,
            })
            .await;

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

    send_msg(send, &TransferMsg::TransferComplete).await?;

    let _ = event_tx
        .send(AppEvent::TransferCompleted(file_info.file_name.clone()))
        .await;

    Ok(())
}
