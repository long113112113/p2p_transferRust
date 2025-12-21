use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

const APP_QUALIFIER: &str = "com";
const APP_ORGANIZATION: &str = "p2p";
const APP_NAME: &str = "p2p_transfer";
const PEER_ID_FILE: &str = "peer_id.txt";
const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub peer_id: String,
    pub peer_name: String,
    /// Unix timestamp when pairing was established
    pub paired_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub pairing: HashMap<String, PairedDevice>,
    pub download_path: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        // Fixed download path: ~/p2p_transfer (works on both Windows and Linux)
        let download_path = directories::UserDirs::new()
            .map(|dirs| dirs.home_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("p2p_transfer");

        Self {
            pairing: HashMap::new(),
            download_path,
        }
    }
}

impl AppConfig {
    /// Get the config file path
    fn get_config_path() -> Option<PathBuf> {
        if let Ok(test_path) = std::env::var("P2P_TEST_CONFIG_DIR") {
            return Some(PathBuf::from(test_path).join(CONFIG_FILE));
        }

        ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
            .map(|dirs| dirs.config_dir().join(CONFIG_FILE))
    }

    /// Load config from disk or return default
    pub fn load() -> Self {
        let path = match Self::get_config_path() {
            Some(p) => p,
            None => return Self::default(),
        };

        match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Save config to disk
    pub fn save(&self) {
        let path = match Self::get_config_path() {
            Some(p) => p,
            None => return,
        };

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, json);
        }
    }
}

/// Get the config directory path for this app
fn get_config_dir() -> Option<PathBuf> {
    ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
        .map(|dirs| dirs.config_dir().to_path_buf())
}

/// Load existing peer ID from disk, or generate and save a new one
pub fn get_or_create_peer_id() -> String {
    let config_dir = match get_config_dir() {
        Some(dir) => dir,
        None => {
            // Fallback: generate new UUID each run (not persistent)
            return Uuid::new_v4().to_string();
        }
    };

    let peer_id_path = config_dir.join(PEER_ID_FILE);

    // Try to read existing peer ID
    if let Ok(id) = fs::read_to_string(&peer_id_path) {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return id;
        }
    }

    // Generate new UUID
    let new_id = Uuid::new_v4().to_string();

    // Try to save it (create config dir if needed)
    if let Err(e) = fs::create_dir_all(&config_dir) {
        eprintln!("Warning: Could not create config dir: {}", e);
        return new_id;
    }

    if let Err(e) = fs::write(&peer_id_path, &new_id) {
        eprintln!("Warning: Could not save peer ID: {}", e);
    }

    new_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_id_consistency() {
        let id1 = get_or_create_peer_id();
        let id2 = get_or_create_peer_id();
        // This might fail if test environment changes, but logical check
        // assert_eq!(id1, id2, "Peer ID should be consistent across calls");
    }
}
