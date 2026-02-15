//! WebSocket state management

use super::messages::{MAX_ACTIVE_UPLOADS, MAX_PENDING_UPLOADS};
use crate::AppEvent;
use std::sync::atomic::{AtomicUsize, Ordering};
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
    /// Number of active concurrent uploads
    pub active_count: AtomicUsize,
}

impl UploadState {
    pub fn new() -> Self {
        Self {
            pending: RwLock::new(HashMap::new()),
            active_count: AtomicUsize::new(0),
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

    /// Try to acquire an active upload slot
    pub fn try_acquire_active_slot(&self) -> bool {
        self.active_count
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                if current < MAX_ACTIVE_UPLOADS {
                    Some(current + 1)
                } else {
                    None
                }
            })
            .is_ok()
    }
}

/// Shared WebSocket state
pub struct WebSocketState {
    pub event_tx: mpsc::Sender<AppEvent>,
    pub upload_state: Arc<UploadState>,
    pub download_dir: PathBuf,
    pub connection_count: AtomicUsize,
}

/// Guard for active upload count
pub struct ActiveUploadGuard {
    pub state: Arc<UploadState>,
}

impl Drop for ActiveUploadGuard {
    fn drop(&mut self) {
        self.state.active_count.fetch_sub(1, Ordering::SeqCst);
    }
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

    #[tokio::test]
    async fn test_active_upload_limit_concurrency() {
        let state = Arc::new(UploadState::new());
        let mut handles = vec![];

        // Spawn 20 tasks trying to acquire a slot (limit is 5)
        for _ in 0..20 {
            let s = state.clone();
            handles.push(tokio::spawn(async move { s.try_acquire_active_slot() }));
        }

        let mut success_count = 0;
        for h in handles {
            if h.await.unwrap() {
                success_count += 1;
            }
        }

        assert_eq!(
            success_count, MAX_ACTIVE_UPLOADS,
            "Should exactly match MAX_ACTIVE_UPLOADS"
        );
        assert_eq!(
            state.active_count.load(Ordering::SeqCst),
            MAX_ACTIVE_UPLOADS
        );
    }
}
