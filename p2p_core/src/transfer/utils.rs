use crate::AppEvent;
use std::path::Path;
use std::time::Instant;
use tokio::sync::mpsc;

/// Sanitize file name to prevent path traversal
pub fn sanitize_file_name(file_name: &str) -> String {
    Path::new(file_name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown_file")
        .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_file_name() {
        assert_eq!(sanitize_file_name("normal_file.txt"), "normal_file.txt");
        assert_eq!(sanitize_file_name("path/to/file.txt"), "file.txt");
        assert_eq!(sanitize_file_name("/absolute/path/to/file.txt"), "file.txt");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name(".."), "unknown_file");
        assert_eq!(sanitize_file_name(""), "unknown_file");
        assert_eq!(sanitize_file_name("foo/../bar.txt"), "bar.txt");
    }
}
