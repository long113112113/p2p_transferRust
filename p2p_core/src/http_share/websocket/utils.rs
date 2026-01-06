//! WebSocket utility functions

use super::messages::{ClientMessage, MAX_FILENAME_LENGTH, MAX_FILE_SIZE};
use super::state::UploadState;
use axum::extract::ws::{Message, WebSocket};
use futures_util::StreamExt;

/// Wait for file_info message
pub async fn wait_for_file_info(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<(String, u64)> {
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
}
