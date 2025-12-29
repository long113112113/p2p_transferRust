use anyhow::{Context, Result};
use iroh::SecretKey;
use std::path::PathBuf;
use tokio::fs;

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

            fs::write(&key_path, secret_key.to_bytes())
                .await
                .context("Failed to save secret key")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = fs::metadata(&key_path)
                    .await
                    .context("Failed to get metadata")?
                    .permissions();
                perms.set_mode(0o600);
                fs::set_permissions(&key_path, perms)
                    .await
                    .context("Failed to set permissions")?;
            }

            Ok(secret_key)
        }
    }

    /// Synchronous version for use in sync contexts (blocks on tokio runtime)
    pub fn load_or_generate_sync(&self) -> Result<SecretKey> {
        let key_path = self.config_dir.join(KEY_FILE_NAME);

        if key_path.exists() {
            tracing::info!("Loading existing identity from {:?}", key_path);
            let key_bytes = std::fs::read(&key_path).context("Failed to read secret key file")?;
            let bytes: [u8; 32] = key_bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid secret key length in file"))?;

            Ok(SecretKey::from_bytes(&bytes))
        } else {
            let secret_key = SecretKey::generate(&mut rand::rng());
            if let Some(parent) = key_path.parent() {
                std::fs::create_dir_all(parent).context("Failed to create config directory")?;
            }

            std::fs::write(&key_path, secret_key.to_bytes())
                .context("Failed to save secret key")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&key_path)
                    .context("Failed to get metadata")?
                    .permissions();
                perms.set_mode(0o600);
                std::fs::set_permissions(&key_path, perms)
                    .context("Failed to set permissions")?;
            }

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
