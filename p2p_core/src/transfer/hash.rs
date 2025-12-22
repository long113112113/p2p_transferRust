use anyhow::Result;
use blake3::Hasher;
use std::path::PathBuf;

/// Compute BLAKE3 hash of a file using parallelism
pub async fn compute_file_hash(file_path: &PathBuf) -> Result<String> {
    let path = file_path.clone();

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path)?;
        let metadata = file.metadata()?;
        let len = metadata.len();

        let mut hasher = Hasher::new();

        if len > 0 {
            // Use memory mapping for faster file reading and parallel hashing
            // SAFETY: Memory mapping is unsafe because external modifications to the file
            // while mapped can lead to undefined behavior. In this local transfer context,
            // we assume the file is not being modified concurrently.
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
