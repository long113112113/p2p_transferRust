use directories::ProjectDirs;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

const APP_QUALIFIER: &str = "com";
const APP_ORGANIZATION: &str = "p2p";
const APP_NAME: &str = "p2p_transfer";
const PEER_ID_FILE: &str = "peer_id.txt";

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
        assert_eq!(id1, id2, "Peer ID should be consistent across calls");
    }
}
