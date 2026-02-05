use crate::AppEvent;
use crate::transfer::constants::{MAX_FILE_SIZE, MAX_FILENAME_LENGTH};
use anyhow::Result;
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::path::Path;
use std::time::Instant;
use tokio::fs::{File, OpenOptions};
use tokio::sync::mpsc;

/// Validate file info against security limits (size and name length)
pub fn validate_transfer_info(file_name: &str, file_size: u64) -> Result<()> {
    if file_size > MAX_FILE_SIZE {
        return Err(anyhow::anyhow!(
            "File rejected: {} ({} GB) exceeds maximum allowed size of {} GB",
            file_name,
            file_size / (1024 * 1024 * 1024),
            MAX_FILE_SIZE / (1024 * 1024 * 1024)
        ));
    }

    if file_name.len() > MAX_FILENAME_LENGTH {
        return Err(anyhow::anyhow!(
            "File rejected: Filename too long ({} chars, max {})",
            file_name.len(),
            MAX_FILENAME_LENGTH
        ));
    }
    Ok(())
}

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
    // 1. Get the last component using string splitting to be OS-agnostic
    // Split by / and \ and take the last part
    let file_name = file_name
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(file_name);

    // 2. Filter characters (allow alphanum, space, ., -, _, (, ), [, ])
    // Disallow control characters, nulls, etc.
    // Also explicitly remove any remaining / or \ just in case
    let sanitized: String = file_name
        .chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
        .collect();

    // 3. Trim
    let sanitized = sanitized.trim().to_string();

    // 4. Handle reserved names (Windows) - Optional but good for defense in depth
    // CON, PRN, AUX, NUL, COM1-9, LPT1-9
    let upper = sanitized.to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    // Check if it exactly matches or matches name with extension (e.g. CON.txt is invalid on Windows too)
    let stem = std::path::Path::new(&sanitized)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();

    if reserved.contains(&stem.as_str()) || reserved.contains(&upper.as_str()) {
        return format!("_{}", sanitized);
    }

    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        return "unknown_file.bin".to_string();
    }

    sanitized
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
    fn test_validate_transfer_info() {
        // Valid case
        assert!(validate_transfer_info("valid.txt", 1024).is_ok());

        // Invalid size
        assert!(validate_transfer_info("huge.txt", MAX_FILE_SIZE + 1).is_err());

        // Invalid name length
        let long_name = "a".repeat(MAX_FILENAME_LENGTH + 1);
        assert!(validate_transfer_info(&long_name, 1024).is_err());
    }

    #[test]
    fn test_sanitize_file_name() {
        assert_eq!(sanitize_file_name("normal_file.txt"), "normal_file.txt");
        assert_eq!(sanitize_file_name("path/to/file.txt"), "file.txt");
        assert_eq!(sanitize_file_name("/absolute/path/to/file.txt"), "file.txt");
        // Cross-platform separators
        assert_eq!(
            sanitize_file_name("path\\to\\windows\\file.txt"),
            "file.txt"
        );
        assert_eq!(sanitize_file_name("mixed/path\\to/file.txt"), "file.txt");

        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name(".."), "unknown_file.bin");
        assert_eq!(sanitize_file_name("."), "unknown_file.bin");
        assert_eq!(sanitize_file_name(""), "unknown_file.bin");
        assert_eq!(sanitize_file_name("   "), "unknown_file.bin");
        assert_eq!(sanitize_file_name("foo/../bar.txt"), "bar.txt");

        // Control characters
        assert_eq!(sanitize_file_name("file\nname.txt"), "filename.txt");
        assert_eq!(sanitize_file_name("file\0name.txt"), "filename.txt");

        // Reserved names
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("con.txt"), "_con.txt");
        assert_eq!(sanitize_file_name("LPT1"), "_LPT1");
        assert_eq!(sanitize_file_name("aux"), "_aux");

        // Unicode
        assert_eq!(sanitize_file_name("文件.txt"), "文件.txt");
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
