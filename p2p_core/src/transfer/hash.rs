use anyhow::Result;
use blake3::Hasher;

/// Compute BLAKE3 hash of a file using parallelism
pub async fn compute_file_hash(file_path: &std::path::Path) -> Result<String> {
    let path = file_path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path)?;
        let metadata = file.metadata()?;
        let len = metadata.len();

        let mut hasher = Hasher::new();

        if len > 0 {
            match unsafe { memmap2::Mmap::map(&file) } {
                Ok(mmap) => {
                    hasher.update_rayon(&mmap);
                }
                Err(_) => {
                    // Fallback to standard reading if mmap fails
                    use std::io::Read;
                    let mut buffer = vec![0u8; 65536]; // 64KB buffer
                    let mut reader = std::io::BufReader::new(file);
                    loop {
                        let n = reader.read(&mut buffer)?;
                        if n == 0 {
                            break;
                        }
                        hasher.update(&buffer[..n]);
                    }
                }
            }
        }

        Ok(hasher.finalize().to_hex().to_string())
    })
    .await?
}
