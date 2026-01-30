pub const TRANSFER_PORT: u16 = 9000;

pub const BUFFER_SIZE: usize = 16 * 1024 * 1024;

/// Maximum file size (10 GB)
pub const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024;

/// Maximum filename length (255 bytes)
pub const MAX_FILENAME_LENGTH: usize = 255;

/// Maximum protocol message size (64KB) to prevent DoS via allocation
pub const MAX_MSG_SIZE: usize = 64 * 1024;
