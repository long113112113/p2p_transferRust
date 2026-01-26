//! WebSocket utility functions

use super::messages::{ClientMessage, HANDSHAKE_TIMEOUT_SECS, MAX_FILE_SIZE, MAX_FILENAME_LENGTH};
use super::state::UploadState;
use axum::extract::ws::{Message, WebSocket};
use futures_util::StreamExt;
use std::path::Path;
use std::time::Duration;
use tokio::fs::{File, OpenOptions};
use tokio::time::timeout;

/// Create a file with secure permissions (0o600 on Unix)
pub async fn create_secure_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    options.mode(0o600);

    options.open(path).await
}

/// Wait for file_info message
pub async fn wait_for_file_info(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<(String, u64)> {
    let duration = Duration::from_secs(HANDSHAKE_TIMEOUT_SECS);

    let result = timeout(duration, async {
        while let Some(msg) = receiver.next().await {
            if let Ok(Message::Text(text)) = msg
                && let Ok(ClientMessage::FileInfo {
                    file_name,
                    file_size,
                }) = serde_json::from_str(&text)
            {
                return Some((file_name, file_size));
            }
        }
        None
    })
    .await;

    match result {
        Ok(Some(info)) => Some(info),
        _ => None, // Timeout or stream ended
    }
}

/// Validate file info against security constraints
pub fn validate_file_info(file_name: &str, file_size: u64) -> Result<(), String> {
    if file_name.len() > MAX_FILENAME_LENGTH {
        return Err(format!(
            "Filename too long (max {} characters)",
            MAX_FILENAME_LENGTH
        ));
    }

    if file_size > MAX_FILE_SIZE {
        return Err(format!(
            "File too large (max {} GB)",
            MAX_FILE_SIZE / (1024 * 1024 * 1024)
        ));
    }

    Ok(())
}

/// Clean up pending upload
pub async fn cleanup_pending(state: &UploadState, request_id: &str) {
    let mut pending = state.pending.write().await;
    pending.remove(request_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_file_info() {
        // Valid
        assert!(validate_file_info("test.txt", 1024).is_ok());

        // Invalid name length
        let long_name = "a".repeat(MAX_FILENAME_LENGTH + 1);
        assert!(validate_file_info(&long_name, 1024).is_err());

        // Invalid file size
        assert!(validate_file_info("test.txt", MAX_FILE_SIZE + 1).is_err());
    }

    #[tokio::test]
    async fn test_create_secure_file_permissions() {
        let temp_dir = std::env::temp_dir();
        let file_path = temp_dir.join(format!("secure_test_{}.txt", uuid::Uuid::new_v4()));

        // Cleanup first just in case
        let _ = tokio::fs::remove_file(&file_path).await;

        let _file = create_secure_file(&file_path)
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
