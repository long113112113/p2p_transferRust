//! Pairing management for trusted devices.
//!
//! Stores paired endpoint IDs with 24-hour expiry.

use crate::config::{AppConfig, PairedDevice};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

/// Pairing expires after 24 hours
const PAIRING_EXPIRY_SECS: u64 = 24 * 60 * 60;

/// Maximum concurrent pairing attempts
const MAX_PAIRING_ATTEMPTS: usize = 3;

/// Active pairing attempts counter
static ACTIVE_PAIRING_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);

/// Guard to track an active pairing attempt
pub struct PairingGuard;

impl PairingGuard {
    /// Try to acquire a pairing slot. Returns None if limit reached.
    pub fn try_acquire() -> Option<Self> {
        let mut count = ACTIVE_PAIRING_ATTEMPTS.load(Ordering::Relaxed);
        loop {
            if count >= MAX_PAIRING_ATTEMPTS {
                return None;
            }
            match ACTIVE_PAIRING_ATTEMPTS.compare_exchange_weak(
                count,
                count + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Some(Self),
                Err(x) => count = x,
            }
        }
    }
}

impl Drop for PairingGuard {
    fn drop(&mut self) {
        ACTIVE_PAIRING_ATTEMPTS.fetch_sub(1, Ordering::Relaxed);
    }
}

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

    #[test]
    fn test_concurrency_limit() {
        // Clear state just in case
        ACTIVE_PAIRING_ATTEMPTS.store(0, Ordering::Relaxed);

        // Acquire max guards
        let g1 = PairingGuard::try_acquire();
        assert!(g1.is_some());

        let g2 = PairingGuard::try_acquire();
        assert!(g2.is_some());

        let g3 = PairingGuard::try_acquire();
        assert!(g3.is_some());

        // Should fail now
        let g4 = PairingGuard::try_acquire();
        assert!(g4.is_none());

        // Drop one
        drop(g1);

        // Should succeed now
        let g5 = PairingGuard::try_acquire();
        assert!(g5.is_some());

        // Should fail again
        let g6 = PairingGuard::try_acquire();
        assert!(g6.is_none());

        // Cleanup
        drop(g2);
        drop(g3);
        drop(g5);

        assert_eq!(ACTIVE_PAIRING_ATTEMPTS.load(Ordering::Relaxed), 0);
    }
}
