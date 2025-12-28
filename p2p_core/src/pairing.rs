//! Pairing management for trusted devices.
//!
//! Stores paired endpoint IDs with 24-hour expiry.

use crate::config::{AppConfig, PairedDevice};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

/// Pairing expires after 24 hours
const PAIRING_EXPIRY_SECS: u64 = 24 * 60 * 60;

fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub fn is_paired(endpoint_id: &str) -> bool {
    let config = AppConfig::load();

    if let Some(device) = config.pairing.get(endpoint_id) {
        let now = now_timestamp();
        let elapsed = now.saturating_sub(device.paired_at);
        return elapsed < PAIRING_EXPIRY_SECS;
    }

    false
}

pub fn add_pairing(endpoint_id: &str, peer_name: &str) {
    let mut config = AppConfig::load();

    config.pairing.insert(
        endpoint_id.to_string(),
        PairedDevice {
            endpoint_id: endpoint_id.to_string(),
            peer_name: peer_name.to_string(),
            paired_at: now_timestamp(),
        },
    );

    remove_expired(&mut config);

    config.save();
}

pub fn remove_pairing(endpoint_id: &str) {
    let mut config = AppConfig::load();
    config.pairing.remove(endpoint_id);
    config.save();
}

fn remove_expired(config: &mut AppConfig) {
    let now = now_timestamp();
    config.pairing.retain(|_, device| {
        let elapsed = now.saturating_sub(device.paired_at);
        elapsed < PAIRING_EXPIRY_SECS
    });
}

pub fn get_all_pairings() -> Vec<(String, String)> {
    let mut config = AppConfig::load();
    remove_expired(&mut config);

    config
        .pairing
        .values()
        .map(|d| (d.endpoint_id.clone(), d.peer_name.clone()))
        .collect()
}

pub fn generate_verification_code() -> String {
    // Securely generate a random number for the verification code
    // We use Uuid::new_v4() which relies on a CSPRNG (getrandom)
    let uuid = Uuid::new_v4();
    let bytes = uuid.as_bytes();
    // Use the first 4 bytes to form a u32
    // Use from_ne_bytes for random number; endianness indifferent
    let val = u32::from_ne_bytes(bytes[0..4].try_into().unwrap_or([0; 4]));
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
