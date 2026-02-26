pub const TRANSFER_PORT: u16 = 9000;

pub const BUFFER_SIZE: usize = 16 * 1024 * 1024;

/// Maximum file size (10 GB)
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024;

/// Maximum filename length (255 bytes)
pub const MAX_FILENAME_LENGTH: usize = 255;

/// Maximum protocol message size (64KB) to prevent DoS via allocation
pub const MAX_MSG_SIZE: usize = 64 * 1024;

/// Timeout for pairing verification code input
pub const DEFAULT_PAIRING_TIMEOUT_SECS: u64 = 60;

pub fn get_pairing_timeout() -> std::time::Duration {
    // Check environment variable for override (useful for tests)
    let secs = std::env::var("P2P_PAIRING_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PAIRING_TIMEOUT_SECS);
    std::time::Duration::from_secs(secs)
}
