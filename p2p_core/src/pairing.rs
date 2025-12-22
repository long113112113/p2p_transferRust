//! Pairing management for trusted devices.
//!
//! Stores paired peer IDs with 24-hour expiry.

use crate::config::{AppConfig, PairedDevice};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Pairing expires after 24 hours
const PAIRING_EXPIRY_SECS: u64 = 24 * 60 * 60;

/// Get current Unix timestamp
fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

/// Check if a peer is already paired and not expired
pub fn is_paired(peer_id: &str) -> bool {
    let config = AppConfig::load();

    if let Some(device) = config.pairing.get(peer_id) {
        let now = now_timestamp();
        let elapsed = now.saturating_sub(device.paired_at);
        return elapsed < PAIRING_EXPIRY_SECS;
    }

    false
}

/// Add a new pairing (or update existing)
pub fn add_pairing(peer_id: &str, peer_name: &str) {
    let mut config = AppConfig::load();

    config.pairing.insert(
        peer_id.to_string(),
        PairedDevice {
            peer_id: peer_id.to_string(),
            peer_name: peer_name.to_string(),
            paired_at: now_timestamp(),
        },
    );

    // Clean up expired pairings
    remove_expired(&mut config);

    config.save();
}

/// Remove a specific pairing
pub fn remove_pairing(peer_id: &str) {
    let mut config = AppConfig::load();
    config.pairing.remove(peer_id);
    config.save();
}

/// Remove all expired pairings
fn remove_expired(config: &mut AppConfig) {
    let now = now_timestamp();
    config.pairing.retain(|_, device| {
        let elapsed = now.saturating_sub(device.paired_at);
        elapsed < PAIRING_EXPIRY_SECS
    });
}

/// Get all currently valid pairings (for debugging/UI)
pub fn get_all_pairings() -> Vec<(String, String)> {
    let mut config = AppConfig::load();
    remove_expired(&mut config);

    config
        .pairing
        .values()
        .map(|d| (d.peer_id.clone(), d.peer_name.clone()))
        .collect()
}

/// Generate a random 4-digit verification code
pub fn generate_verification_code() -> String {
    use uuid::Uuid;

    // Use a cryptographically secure random number generator via UUID v4
    let uuid = Uuid::new_v4();
    let bytes = uuid.as_bytes();

    // Use first 4 bytes to generate a u32, then modulo 10000
    // UUID v4 uses a CSPRNG, so this is secure enough for short verification codes
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&bytes[0..4]);
    let val = u32::from_ne_bytes(arr);

    let code = val % 10000;
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
