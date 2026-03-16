use anyhow::{Context, Result};
use iroh::SecretKey;
use std::path::PathBuf;
use tokio::{fs, io::AsyncWriteExt};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

const KEY_FILE_NAME: &str = "node_secret.key";

/// Quản lý việc lưu và tải SecretKey để giữ Node ID cố định
pub struct IdentityManager {
    config_dir: PathBuf,
}

impl IdentityManager {
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Load existing SecretKey or generate a new one
    pub async fn load_or_generate(&self) -> Result<SecretKey> {
        let key_path = self.config_dir.join(KEY_FILE_NAME);

        if key_path.exists() {
            tracing::info!("Loading existing identity from {:?}", key_path);
            let key_bytes = fs::read(&key_path)
                .await
                .context("Failed to read secret key file")?;

            // Check if existing file has wrong permissions and fix them
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&key_path).await?.permissions();
                if perms.mode() & 0o777 != 0o600 {
                    perms.set_mode(0o600);
                    fs::set_permissions(&key_path, perms).await?;
                }
            }

            let bytes: [u8; 32] = key_bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid secret key length in file"))?;

            Ok(SecretKey::from_bytes(&bytes))
        } else {
            let secret_key = SecretKey::generate(&mut rand::rng());
            if let Some(parent) = key_path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .context("Failed to create config directory")?;
            }

            // Use OpenOptions to set file permissions to 600 (read/write only by owner)
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);

            #[cfg(unix)]
            options.mode(0o600);

            let mut file = options
                .open(&key_path)
                .await
                .context("Failed to open secret key file for writing")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = file.metadata().await?.permissions();
                if perms.mode() & 0o777 != 0o600 {
                    perms.set_mode(0o600);
                    file.set_permissions(perms).await?;
                }
            }

            file.write_all(&secret_key.to_bytes())
                .await
                .context("Failed to write secret key")?;

            Ok(secret_key)
        }
    }

    /// Synchronous version for use in sync contexts (blocks on tokio runtime)
    pub fn load_or_generate_sync(&self) -> Result<SecretKey> {
        let key_path = self.config_dir.join(KEY_FILE_NAME);

        if key_path.exists() {
            tracing::info!("Loading existing identity from {:?}", key_path);
            let key_bytes = std::fs::read(&key_path).context("Failed to read secret key file")?;

            // Check if existing file has wrong permissions and fix them
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&key_path)?.permissions();
                if perms.mode() & 0o777 != 0o600 {
                    perms.set_mode(0o600);
                    std::fs::set_permissions(&key_path, perms)?;
                }
            }

            let bytes: [u8; 32] = key_bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid secret key length in file"))?;

            Ok(SecretKey::from_bytes(&bytes))
        } else {
            let secret_key = SecretKey::generate(&mut rand::rng());
            if let Some(parent) = key_path.parent() {
                std::fs::create_dir_all(parent).context("Failed to create config directory")?;
            }

            // Use OpenOptions to set file permissions to 600 (read/write only by owner)
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);

            #[cfg(unix)]
            options.mode(0o600);

            use std::io::Write;
            let mut file = options
                .open(&key_path)
                .context("Failed to open secret key file for writing")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = file.metadata()?.permissions();
                if perms.mode() & 0o777 != 0o600 {
                    perms.set_mode(0o600);
                    file.set_permissions(perms)?;
                }
            }

            file.write_all(&secret_key.to_bytes())
                .context("Failed to write secret key")?;

            Ok(secret_key)
        }
    }

    pub fn get_key_path(&self) -> PathBuf {
        self.config_dir.join(KEY_FILE_NAME)
    }
}

/// Get the Iroh NodeId as the endpoint ID string
/// This provides a unified identity across LAN and WAN transfers
pub fn get_iroh_endpoint_id() -> String {
    use crate::config::get_config_dir;

    let config_dir = get_config_dir().unwrap_or_else(|| PathBuf::from("."));
    let manager = IdentityManager::new(config_dir);

    match manager.load_or_generate_sync() {
        Ok(secret_key) => {
            let public_key = secret_key.public();
            public_key.to_string()
        }
        Err(e) => {
            tracing::error!("Failed to get Iroh identity: {}", e);
            // Fallback to UUID if Iroh fails
            uuid::Uuid::new_v4().to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_load_or_generate_permissions() {
        let temp_dir = std::env::temp_dir().join(format!("p2p_identity_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&temp_dir).await.unwrap();

        let manager = IdentityManager::new(temp_dir.clone());
        let key_path = manager.get_key_path();

        // 1. Create file with 0o666 (rw-rw-rw-)
        {
            // First we need to generate a valid key to put in there to pass the length check if it loads it
            let secret_key = SecretKey::generate(&mut rand::rng());
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&key_path)
                .await
                .expect("Failed to create initial file");

            file.write_all(&secret_key.to_bytes()).await.unwrap();

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = file.metadata().await.unwrap().permissions();
                perms.set_mode(0o666);
                file.set_permissions(perms).await.unwrap();
            }
        }

        // 2. Overwrite using load_or_generate
        let _key = manager.load_or_generate().await.expect("Failed to load/generate identity");

        // 3. Verify permissions are now 0o600
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&key_path)
                .await
                .expect("Failed to get metadata");
            let permissions = metadata.permissions();
            assert_eq!(
                permissions.mode() & 0o777,
                0o600,
                "File permissions should be reset to 0o600 upon overwrite"
            );
        }

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir).await;
    }

    #[test]
    fn test_load_or_generate_sync_permissions() {
        let temp_dir = std::env::temp_dir().join(format!("p2p_identity_test_sync_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let manager = IdentityManager::new(temp_dir.clone());
        let key_path = manager.get_key_path();

        // 1. Create file with 0o666 (rw-rw-rw-)
        {
            // First we need to generate a valid key to put in there to pass the length check if it loads it
            use std::io::Write;
            let secret_key = SecretKey::generate(&mut rand::rng());
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&key_path)
                .expect("Failed to create initial file");

            file.write_all(&secret_key.to_bytes()).unwrap();

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = file.metadata().unwrap().permissions();
                perms.set_mode(0o666);
                file.set_permissions(perms).unwrap();
            }
        }

        // 2. Overwrite using load_or_generate_sync
        let _key = manager.load_or_generate_sync().expect("Failed to load/generate identity");

        // 3. Verify permissions are now 0o600
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&key_path)
                .expect("Failed to get metadata");
            let permissions = metadata.permissions();
            assert_eq!(
                permissions.mode() & 0o777,
                0o600,
                "File permissions should be reset to 0o600 upon overwrite"
            );
        }

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
