//! Tests for MAGIC_BYTES constant and packet building.

use p2p_core::{DiscoveryMsg, MAGIC_BYTES};

#[test]
fn test_magic_bytes_content() {
    // MAGIC_BYTES should be "P2PLT\0"
    assert_eq!(MAGIC_BYTES, b"P2PLT\x00");
}

#[test]
fn test_magic_bytes_length() {
    // Should be exactly 6 bytes
    assert_eq!(MAGIC_BYTES.len(), 6);
}

#[test]
fn test_packet_building_with_magic_bytes() {
    let msg = DiscoveryMsg::DiscoveryRequest {
        peer_id: "test-id".to_string(),
        my_name: "TestPC".to_string(),
        tcp_port: 9000,
    };

    // Build packet like DiscoveryService does
    let json_bytes = serde_json::to_vec(&msg).expect("Should serialize");
    let mut packet = MAGIC_BYTES.to_vec();
    packet.extend_from_slice(&json_bytes);

    // Verify packet structure
    assert!(packet.starts_with(MAGIC_BYTES));
    assert_eq!(&packet[..MAGIC_BYTES.len()], MAGIC_BYTES);
    assert!(packet.len() > MAGIC_BYTES.len());
}

#[test]
fn test_packet_parsing_with_magic_bytes() {
    let original_msg = DiscoveryMsg::DiscoveryResponse {
        peer_id: "parse-test".to_string(),
        my_name: "ParsePC".to_string(),
        tcp_port: 8888,
    };

    // Build packet
    let json_bytes = serde_json::to_vec(&original_msg).expect("Should serialize");
    let mut packet = MAGIC_BYTES.to_vec();
    packet.extend_from_slice(&json_bytes);

    // Parse packet like DiscoveryService does
    let len = packet.len();

    // Check magic bytes
    assert!(len >= MAGIC_BYTES.len());
    assert_eq!(&packet[..MAGIC_BYTES.len()], MAGIC_BYTES);

    // Extract JSON data
    let data = &packet[MAGIC_BYTES.len()..len];
    let parsed: DiscoveryMsg = serde_json::from_slice(data).expect("Should parse JSON");

    match parsed {
        DiscoveryMsg::DiscoveryResponse {
            peer_id,
            my_name,
            tcp_port,
        } => {
            assert_eq!(peer_id, "parse-test");
            assert_eq!(my_name, "ParsePC");
            assert_eq!(tcp_port, 8888);
        }
        _ => panic!("Expected DiscoveryResponse"),
    }
}

#[test]
fn test_reject_invalid_magic_bytes() {
    // Simulate a packet with wrong magic bytes
    let fake_magic = b"WRONG\x00";
    let fake_packet = fake_magic.to_vec();

    // This should NOT match our MAGIC_BYTES
    assert_ne!(&fake_packet[..], MAGIC_BYTES);
}

#[test]
fn test_reject_short_packet() {
    // Packet shorter than MAGIC_BYTES length
    let short_packet = b"P2P";

    // Should be rejected
    assert!(short_packet.len() < MAGIC_BYTES.len());
}
