use anyhow::Result;
use blake3::Hasher;
use std::path::PathBuf;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use super::constants::BUFFER_SIZE;

/// Compute BLAKE3 hash of a file
pub async fn compute_file_hash(file_path: &PathBuf) -> Result<String> {
    let mut file = File::open(file_path).await?;
    let mut hasher = Hasher::new();
    let mut buffer = vec![0u8; BUFFER_SIZE];

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}
