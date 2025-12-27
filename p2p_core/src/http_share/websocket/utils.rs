//! WebSocket utility functions

use super::messages::ClientMessage;
use super::state::UploadState;
use axum::extract::ws::{Message, WebSocket};
use futures_util::StreamExt;

/// Wait for file_info message
pub async fn wait_for_file_info(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<(String, u64)> {
    while let Some(msg) = receiver.next().await {
        if let Ok(Message::Text(text)) = msg {
            if let Ok(ClientMessage::FileInfo {
                file_name,
                file_size,
            }) = serde_json::from_str(&text)
            {
                return Some((file_name, file_size));
            }
        }
    }
    None
}

/// Clean up pending upload
pub async fn cleanup_pending(state: &UploadState, request_id: &str) {
    let mut pending = state.pending.write().await;
    pending.remove(request_id);
}
