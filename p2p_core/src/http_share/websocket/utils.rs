//! WebSocket utility functions

use super::messages::{ClientMessage, HANDSHAKE_TIMEOUT_SECS};
use super::state::UploadState;
use crate::transfer::constants::{MAX_FILE_SIZE, MAX_FILENAME_LENGTH};
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

    let file = options.open(path).await?;

    // Explicitly set permissions to ensure security even if file already existed
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = file.metadata().await?.permissions();
        if perms.mode() & 0o777 != 0o600 {
            perms.set_mode(0o600);
            file.set_permissions(perms).await?;
        }
    }

    Ok(file)
}

/// Wait for file_info message
pub async fn wait_for_file_info(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<(String, u64)> {
    let duration = Duration::from_secs(HANDSHAKE_TIMEOUT_SECS);

    let result = timeout(duration, async {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    // Enforce strict protocol: first message must be valid FileInfo
                    match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(ClientMessage::FileInfo {
                            file_name,
                            file_size,
                        }) => return Some((file_name, file_size)),
                        _ => return None, // Invalid JSON or wrong message type
                    }
                }
                Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => continue,
                _ => return None, // Unexpected message type (Binary, Close, etc.) or Error
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

    #[tokio::test]
    async fn test_create_secure_file_overwrite_permissions() {
        let temp_dir = std::env::temp_dir();
        // Use a unique name
        let file_path = temp_dir.join(format!("secure_ws_overwrite_test_{}.txt", uuid::Uuid::new_v4()));

        // 1. Create file with 0o666 (rw-rw-rw-)
        {
            let file = tokio::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&file_path)
                .await
                .expect("Failed to create initial file");

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = file.metadata().await.unwrap().permissions();
                perms.set_mode(0o666);
                file.set_permissions(perms).await.unwrap();
            }
        }

        // 2. Overwrite using create_secure_file
        let _file = create_secure_file(&file_path)
            .await
            .expect("Failed to create secure file");

        // 3. Verify permissions are now 0o600
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = tokio::fs::metadata(&file_path)
                .await
                .expect("Failed to get metadata");
            let permissions = metadata.permissions();
            assert_eq!(
                permissions.mode() & 0o777,
                0o600,
                "File permissions should be reset to 0o600 upon overwrite"
            );
        }

        // Cleanup
        let _ = tokio::fs::remove_file(&file_path).await;
    }
}
