//! WebSocket message types and constants

use serde::{Deserialize, Serialize};

/// Chunk size for binary transfer (256KB - optimized for LAN)
pub const CHUNK_SIZE: usize = 256 * 1024;

/// Timeout for user response (60 seconds)
pub const USER_RESPONSE_TIMEOUT_SECS: u64 = 60;

/// Timeout for WebSocket handshake (10 seconds)
pub const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

/// Maximum filename length (255 bytes)
pub const MAX_FILENAME_LENGTH: usize = 255;

/// Maximum file size (10 GB)
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024;

/// Messages from client to server
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Initial file info before upload
    FileInfo { file_name: String, file_size: u64 },
}

/// Messages from server to client
#[derive(Debug, Serialize)]
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
        assert_eq!(MAX_FILENAME_LENGTH, 255);
        assert!(MAX_FILE_SIZE > 0);
    }
}
