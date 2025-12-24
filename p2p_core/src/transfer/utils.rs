use crate::AppEvent;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::mpsc;

/// Sanitize filename to prevent path traversal
pub fn sanitize_filename(filename: &str) -> PathBuf {
    let path = Path::new(filename);
    // Get the file name component (strips directories)
    match path.file_name() {
        Some(name) => {
            let name_str = name.to_string_lossy();
            // Additional check for empty or dot-only names
            if name_str == "." || name_str == ".." || name_str.is_empty() {
                PathBuf::from("unnamed_file")
            } else {
                PathBuf::from(name_str.into_owned())
            }
        }
        None => PathBuf::from("unnamed_file"),
    }
}

/// Format transfer speed from bytes and elapsed time
pub fn format_transfer_speed(bytes_transferred: u64, elapsed_secs: f64) -> String {
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

/// Report transfer progress to the event channel
pub async fn report_progress(
    event_tx: &mpsc::Sender<AppEvent>,
    file_name: &str,
    bytes_done: u64,
    total_bytes: u64,
    start_time: Instant,
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
