//! WebSocket message types and constants

use serde::{Deserialize, Serialize};

/// Chunk size for binary transfer (256KB - optimized for LAN)
pub const CHUNK_SIZE: usize = 256 * 1024;

/// Timeout for user response (60 seconds)
pub const USER_RESPONSE_TIMEOUT_SECS: u64 = 60;

/// Timeout for WebSocket handshake (10 seconds)
pub const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

pub const MAX_PENDING_UPLOADS: usize = 10;

/// Messages from client to server
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Initial file info before upload
    FileInfo { file_name: String, file_size: u64 },
}

/// Messages from server to client
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Upload request accepted
    Accepted { request_id: String },
    /// Upload request rejected
    Rejected { reason: String },
    /// Progress update
    Progress { received_bytes: u64 },
    /// Upload complete
    Complete { saved_path: String },
    /// Error occurred
    Error { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        use crate::transfer::constants::{MAX_FILENAME_LENGTH, MAX_FILE_SIZE};
        assert_eq!(MAX_FILENAME_LENGTH, 255);
        assert!(MAX_FILE_SIZE > 0);
        assert_eq!(MAX_PENDING_UPLOADS, 10);
    }
}
