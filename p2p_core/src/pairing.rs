//! Pairing management for trusted devices.
//!
//! Stores paired peer IDs with 24-hour expiry.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Pairing expires after 24 hours
const PAIRING_EXPIRY_SECS: u64 = 24 * 60 * 60;

/// File name for paired devices storage
const PAIRED_DEVICES_FILE: &str = "paired_devices.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PairedDevice {
    peer_id: String,
    peer_name: String,
    /// Unix timestamp when pairing was established
    paired_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PairingStore {
    devices: HashMap<String, PairedDevice>,
}

/// Get the path to paired devices file
fn get_pairing_file_path() -> Option<PathBuf> {
    if let Ok(test_path) = std::env::var("P2P_TEST_CONFIG_DIR") {
        return Some(PathBuf::from(test_path).join(PAIRED_DEVICES_FILE));
    }
    directories::ProjectDirs::from("com", "p2p", "p2p_transfer")
        .map(|dirs| dirs.config_dir().join(PAIRED_DEVICES_FILE))
}

/// Load pairing store from disk
fn load_store() -> PairingStore {
    let path = match get_pairing_file_path() {
        Some(p) => p,
        None => return PairingStore::default(),
    };

    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => PairingStore::default(),
    }
}

/// Save pairing store to disk
fn save_store(store: &PairingStore) {
    let path = match get_pairing_file_path() {
        Some(p) => p,
        None => return,
    };

    // Ensure config directory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(json) = serde_json::to_string_pretty(store) {
        let _ = fs::write(&path, json);
    }
}

/// Get current Unix timestamp
fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

/// Check if a peer is already paired and not expired
pub fn is_paired(peer_id: &str) -> bool {
    let store = load_store();

    if let Some(device) = store.devices.get(peer_id) {
        let now = now_timestamp();
        let elapsed = now.saturating_sub(device.paired_at);
        return elapsed < PAIRING_EXPIRY_SECS;
    }

    false
}

/// Add a new pairing (or update existing)
pub fn add_pairing(peer_id: &str, peer_name: &str) {
    let mut store = load_store();

    store.devices.insert(
        peer_id.to_string(),
        PairedDevice {
            peer_id: peer_id.to_string(),
            peer_name: peer_name.to_string(),
            paired_at: now_timestamp(),
        },
    );

    // Clean up expired pairings while we're at it
    remove_expired(&mut store);

    save_store(&store);
}

/// Remove a specific pairing
pub fn remove_pairing(peer_id: &str) {
    let mut store = load_store();
    store.devices.remove(peer_id);
    save_store(&store);
}

/// Remove all expired pairings
fn remove_expired(store: &mut PairingStore) {
    let now = now_timestamp();
    store.devices.retain(|_, device| {
        let elapsed = now.saturating_sub(device.paired_at);
        elapsed < PAIRING_EXPIRY_SECS
    });
}

/// Get all currently valid pairings (for debugging/UI)
pub fn get_all_pairings() -> Vec<(String, String)> {
    let mut store = load_store();
    remove_expired(&mut store);

    store
        .devices
        .values()
        .map(|d| (d.peer_id.clone(), d.peer_name.clone()))
        .collect()
}

/// Generate a random 4-digit verification code
pub fn generate_verification_code() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Simple random using timestamp + some variation
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_nanos();

    let code = (seed % 10000) as u32;
    format!("{:04}", code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verification_code_format() {
        let code = generate_verification_code();
        assert_eq!(code.len(), 4);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }
}
