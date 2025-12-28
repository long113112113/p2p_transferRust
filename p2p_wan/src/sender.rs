use anyhow::{Result, anyhow};
use iroh::endpoint::Connection;
use p2p_core::{AppEvent, FileInfo};
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::protocol::{WanTransferMsg, recv_msg, send_msg};

/// Buffer size for file transfer (16MB)
const BUFFER_SIZE: usize = 16 * 1024 * 1024;

/// Send files to a connected peer over WAN
///
/// # Arguments
/// * `connection` - Established iroh connection to the peer
/// * `files` - List of file paths to send
/// * `event_tx` - Channel to send progress events to GUI
pub async fn send_files(
    connection: &Connection,
    files: Vec<PathBuf>,
    event_tx: mpsc::Sender<AppEvent>,
) -> Result<()> {
    let peer_id = connection.remote_id();
    info!("Starting file transfer to peer: {}", peer_id);

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Connected to peer: {}. Starting file transfer...",
            peer_id
        )))
        .await;

    let mut handles = Vec::new();

    for file_path in files.iter() {
        let connection = connection.clone();
        let file_path = file_path.clone();
        let event_tx = event_tx.clone();

        let handle = tokio::spawn(async move {
            if let Err(e) = send_single_file(&connection, &file_path, &event_tx).await {
                error!("Error sending {}: {}", file_path.display(), e);
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

/// Send a single file through the connection
async fn send_single_file(
    connection: &Connection,
    file_path: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    // Open file and get metadata
    let mut file = File::open(file_path).await?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Invalid file name"))?
        .to_string();

    info!("Sending file: {} ({} bytes)", file_name, file_size);
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Sending: {} ({} bytes)",
            file_name, file_size
        )))
        .await;

    // Open bidirectional stream for this file
    let (mut send_stream, mut recv_stream) = connection.open_bi().await?;

    // Create FileInfo and send metadata
    let file_info = FileInfo {
        file_name: file_name.clone(),
        file_size,
        file_path: PathBuf::new(), // Don't expose local path
        file_hash: None,           // Can be added later for verification
    };

    send_msg(
        &mut send_stream,
        &WanTransferMsg::FileMetadata { info: file_info },
    )
    .await?;

    // Wait for resume info from receiver
    let msg = recv_msg(&mut recv_stream).await?;
    let offset = match msg {
        WanTransferMsg::ResumeInfo { offset } => offset,
        WanTransferMsg::Error { message } => {
            return Err(anyhow!("Receiver error: {}", message));
        }
        _ => return Err(anyhow!("Expected ResumeInfo, got {:?}", msg)),
    };

    // Seek to offset if resuming
    if offset > 0 {
        info!("Resuming transfer from offset: {}", offset);
        file.seek(std::io::SeekFrom::Start(offset)).await?;
    }

    // Transfer file data
    let mut sent: u64 = offset;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let start_time = std::time::Instant::now();
    let mut last_progress_update = 0u64;

    // Report initial progress
    report_progress(
        event_tx, &file_name, sent, file_size, start_time, offset, true,
    )
    .await;

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }

        send_stream.write_all(&buffer[..n]).await?;
        sent += n as u64;

        // Report progress every BUFFER_SIZE bytes or when complete
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

    // Wait for receiver confirmation
    match recv_msg(&mut recv_stream).await {
        Ok(WanTransferMsg::TransferComplete) => {
            info!("File transfer confirmed: {}", file_name);
        }
        Ok(WanTransferMsg::Error { message }) => {
            return Err(anyhow!("Transfer failed: {}", message));
        }
        Ok(msg) => {
            error!("Unexpected completion message: {:?}", msg);
        }
        Err(e) => {
            error!("Failed to receive completion ack: {}", e);
        }
    }

    let _ = event_tx.send(AppEvent::TransferCompleted(file_name)).await;
    Ok(())
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
