pub const TRANSFER_PORT: u16 = 9000;

pub const BUFFER_SIZE: usize = 16 * 1024 * 1024;

/// Maximum file size (10 GB)
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024;

/// Maximum filename length (255 bytes)
pub const MAX_FILENAME_LENGTH: usize = 255;

/// Maximum size for protocol messages (64 KB)
pub const MAX_MESSAGE_SIZE: usize = 64 * 1024;
