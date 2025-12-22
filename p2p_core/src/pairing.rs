//! Pairing management for trusted devices.
//!
//! Stores paired peer IDs with 24-hour expiry.

use crate::config::{AppConfig, PairedDevice};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Pairing expires after 24 hours
const PAIRING_EXPIRY_SECS: u64 = 24 * 60 * 60;

fn now_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

pub fn is_paired(peer_id: &str) -> bool {
    let config = AppConfig::load();

    if let Some(device) = config.pairing.get(peer_id) {
        let now = now_timestamp();
        let elapsed = now.saturating_sub(device.paired_at);
        return elapsed < PAIRING_EXPIRY_SECS;
    }

    false
}

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

    remove_expired(&mut config);

    config.save();
}

pub fn remove_pairing(peer_id: &str) {
    let mut config = AppConfig::load();
    config.pairing.remove(peer_id);
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
        .map(|d| (d.peer_id.clone(), d.peer_name.clone()))
        .collect()
}

pub fn generate_verification_code() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

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
