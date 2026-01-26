//! WebSocket state management

use super::messages::MAX_PENDING_UPLOADS;
use crate::AppEvent;
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use tokio::sync::{RwLock, mpsc, oneshot};

/// Pending upload waiting for user response
pub struct PendingUpload {
    pub response_tx: oneshot::Sender<bool>,
}

/// Shared state for upload handling
#[derive(Default)]
pub struct UploadState {
    /// Pending uploads waiting for user approval
    pub pending: RwLock<HashMap<String, PendingUpload>>,
}

impl UploadState {
    pub fn new() -> Self {
        Self {
            pending: RwLock::new(HashMap::new()),
        }
    }

    /// Try to add a pending upload request
    /// Returns false if the pending limit is reached
    pub async fn try_add_request(
        &self,
        request_id: String,
        response_tx: oneshot::Sender<bool>,
    ) -> bool {
        let mut pending = self.pending.write().await;
        if pending.len() >= MAX_PENDING_UPLOADS {
            return false;
        }
        pending.insert(request_id, PendingUpload { response_tx });
        true
    }
}

/// Shared WebSocket state
pub struct WebSocketState {
    pub event_tx: mpsc::Sender<AppEvent>,
    pub upload_state: Arc<UploadState>,
    pub download_dir: PathBuf,
}

/// Respond to an upload request
pub async fn respond_to_upload(state: &UploadState, request_id: &str, accepted: bool) {
    let mut pending = state.pending.write().await;
    if let Some(upload) = pending.remove(request_id) {
        let _ = upload.response_tx.send(accepted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_upload_limit() {
        let state = UploadState::new();

        // Fill up the state
        for i in 0..MAX_PENDING_UPLOADS {
            let (tx, _rx) = oneshot::channel();
            let added = state.try_add_request(format!("req_{}", i), tx).await;
            assert!(added, "Should accept request {}", i);
        }

        // Try to add one more
        let (tx, _rx) = oneshot::channel();
        let added = state.try_add_request("req_overflow".to_string(), tx).await;
        assert!(!added, "Should reject request when full");

        // Remove one
        respond_to_upload(&state, "req_0", true).await;

        // Try adding again
        let (tx, _rx) = oneshot::channel();
        let added = state.try_add_request("req_retry".to_string(), tx).await;
        assert!(added, "Should accept request after space freed");
    }
}
