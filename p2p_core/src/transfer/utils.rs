use crate::AppEvent;
use anyhow::Result;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::path::Path;
use std::time::Instant;
use tokio::fs::{File, OpenOptions};
use tokio::sync::mpsc;

/// Open a file with secure permissions (0o600 on Unix) for writing
pub async fn open_secure_file(path: &Path, offset: u64) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true);

    if offset > 0 {
        options.append(true);
    } else {
        options.create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
    }

    options.open(path).await
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

/// Generate a self-signed certificate for QUIC and HTTPS
pub fn generate_self_signed_cert()
-> Result<(Vec<CertificateDer<'static>>, PrivatePkcs8KeyDer<'static>)> {
    let certified_key = generate_simple_self_signed(vec!["localhost".to_string()])?;
    let key = PrivatePkcs8KeyDer::from(certified_key.signing_key.serialize_der());
    let cert_der = CertificateDer::from(certified_key.cert.der().to_vec());
    Ok((vec![cert_der], key))
}

pub fn sanitize_file_name(file_name: &str) -> String {
    Path::new(file_name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown_file")
        .to_string()
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

    #[tokio::test]
    async fn test_open_secure_file_permissions() {
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(format!("secure_transfer_test_{}.txt", uuid::Uuid::new_v4()));

        // Cleanup first just in case
        let _ = tokio::fs::remove_file(&file_path).await;

        let _file = open_secure_file(&file_path, 0)
            .await
            .expect("Failed to create secure file");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = tokio::fs::metadata(&file_path)
                .await
                .expect("Failed to get metadata");
            let permissions = metadata.permissions();
            // Check that only owner has read/write (0o600)
            // Note: mode() includes file type, so we mask it with 0o777
            assert_eq!(
                permissions.mode() & 0o777,
                0o600,
                "File permissions should be 0o600"
            );
        }

        // Cleanup
        let _ = tokio::fs::remove_file(&file_path).await;
    }
}
