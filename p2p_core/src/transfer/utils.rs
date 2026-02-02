use crate::AppEvent;
use crate::transfer::constants::{MAX_FILENAME_LENGTH, MAX_FILE_SIZE};
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

/// Sanitize file name to prevent path traversal attacks and ensure safety
pub fn sanitize_file_name(file_name: &str) -> String {
    // 1. Split by both / and \ to handle cross-platform paths
    // We take the last component to ignore directories
    let file_name = file_name
        .split(|c| c == '/' || c == '\\')
        .last()
        .unwrap_or("unknown_file");

    if file_name.is_empty() {
        return "unknown_file".to_string();
    }

    // 2. Filter characters
    // Remove control characters and explicit path separators (though split should handle them)
    let mut clean_name: String = file_name.chars()
        .filter(|c| !c.is_control() && *c != '/' && *c != '\\')
        .collect();

    // 3. Check for Windows reserved names
    // See: https://learn.microsoft.com/en-us/windows/win32/fileio/naming-a-file
    let reserved_names = [
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"
    ];

    // Check strict equality ignoring case
    if reserved_names.iter().any(|&r| clean_name.eq_ignore_ascii_case(r)) {
        return "unknown_file".to_string();
    }

    // 4. Check for ".." and "."
    if clean_name == ".." || clean_name == "." || clean_name.trim().is_empty() {
        return "unknown_file".to_string();
    }

    // 5. Truncate to maximum length
    if clean_name.len() > MAX_FILENAME_LENGTH {
        // Try to preserve extension
        if let Some(idx) = clean_name.rfind('.') {
            let ext_len = clean_name.len() - idx;
            // Only try to save extension if it's reasonable length
            if ext_len < 20 && ext_len < MAX_FILENAME_LENGTH {
                let base_len = MAX_FILENAME_LENGTH - ext_len;
                let mut base = clean_name[..idx].to_string();

                // Truncate base safely
                let mut cutoff = base_len;
                while !base.is_char_boundary(cutoff) {
                    cutoff -= 1;
                }
                base.truncate(cutoff);

                base.push_str(&clean_name[idx..]);
                clean_name = base;
            } else {
                // Extension too long, just truncate safely
                let mut cutoff = MAX_FILENAME_LENGTH;
                while !clean_name.is_char_boundary(cutoff) {
                    cutoff -= 1;
                }
                clean_name.truncate(cutoff);
            }
        } else {
            // No extension, truncate safely
            let mut cutoff = MAX_FILENAME_LENGTH;
            while !clean_name.is_char_boundary(cutoff) {
                cutoff -= 1;
            }
            clean_name.truncate(cutoff);
        }
    }

    clean_name
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
    fn test_sanitize_file_name_basic() {
        assert_eq!(sanitize_file_name("normal_file.txt"), "normal_file.txt");
        assert_eq!(sanitize_file_name("path/to/file.txt"), "file.txt");
        assert_eq!(sanitize_file_name("/absolute/path/to/file.txt"), "file.txt");
    }

    #[test]
    fn test_sanitize_file_name_windows_path() {
        assert_eq!(sanitize_file_name("path\\to\\file.txt"), "file.txt");
        assert_eq!(sanitize_file_name("C:\\Windows\\System32\\calc.exe"), "calc.exe");
    }

    #[test]
    fn test_sanitize_file_name_traversal() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("..\\..\\Windows\\System32\\cmd.exe"), "cmd.exe");
        assert_eq!(sanitize_file_name("folder/../file.txt"), "file.txt");
        // Mixed separators
        assert_eq!(sanitize_file_name("folder\\../file.txt"), "file.txt");
    }

    #[test]
    fn test_sanitize_file_name_dangerous() {
        assert_eq!(sanitize_file_name(".."), "unknown_file");
        assert_eq!(sanitize_file_name("."), "unknown_file");
        assert_eq!(sanitize_file_name(""), "unknown_file");
        assert_eq!(sanitize_file_name("/"), "unknown_file");
        assert_eq!(sanitize_file_name("\\"), "unknown_file");
    }

    #[test]
    fn test_sanitize_file_name_reserved() {
        assert_eq!(sanitize_file_name("CON"), "unknown_file");
        assert_eq!(sanitize_file_name("con"), "unknown_file");
        assert_eq!(sanitize_file_name("NUL"), "unknown_file");
        assert_eq!(sanitize_file_name("com1"), "unknown_file");
        // Allowed
        assert_eq!(sanitize_file_name("concert.txt"), "concert.txt");
    }

    #[test]
    fn test_sanitize_file_name_length() {
        let long_name = "a".repeat(300) + ".txt";
        let sanitized = sanitize_file_name(&long_name);
        assert!(sanitized.len() <= MAX_FILENAME_LENGTH);
        assert!(sanitized.ends_with(".txt"));

        let long_no_ext = "a".repeat(300);
        let sanitized_no_ext = sanitize_file_name(&long_no_ext);
        assert!(sanitized_no_ext.len() <= MAX_FILENAME_LENGTH);
    }

    #[test]
    fn test_sanitize_file_name_unicode_truncate() {
        // Create a string that would cause panic if truncated in the middle of a char
        // 🦀 is 4 bytes.
        let crab = "🦀";
        let mut long_unicode = crab.repeat(100); // 400 bytes
        long_unicode.push_str(".txt");

        let sanitized = sanitize_file_name(&long_unicode);
        assert!(sanitized.len() <= MAX_FILENAME_LENGTH);
        assert!(sanitized.ends_with(".txt"));

        // Ensure valid UTF-8 (String does this automatically but good to know we didn't crash)
        // Verify we didn't chop a crab in half
        let base = sanitized.trim_end_matches(".txt");
        assert!(base.chars().last().unwrap() == '🦀');
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
