//! WebSocket state management

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
