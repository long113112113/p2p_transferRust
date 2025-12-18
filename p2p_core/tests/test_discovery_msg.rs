//! Tests for DiscoveryMsg serialization and deserialization.

use p2p_core::DiscoveryMsg;

#[test]
fn test_discovery_request_serialize() {
    let msg = DiscoveryMsg::DiscoveryRequest {
        peer_id: "test-peer-123".to_string(),
        my_name: "TestPC".to_string(),
        tcp_port: 9000,
    };

    let json = serde_json::to_string(&msg).expect("Should serialize");
    assert!(json.contains("DiscoveryRequest"));
    assert!(json.contains("test-peer-123"));
    assert!(json.contains("TestPC"));
    assert!(json.contains("9000"));
}

#[test]
fn test_discovery_request_deserialize() {
    let json = r#"{"DiscoveryRequest":{"peer_id":"peer-456","my_name":"MyPC","tcp_port":8080}}"#;
    let msg: DiscoveryMsg = serde_json::from_str(json).expect("Should deserialize");

    match msg {
        DiscoveryMsg::DiscoveryRequest {
            peer_id,
            my_name,
            tcp_port,
        } => {
            assert_eq!(peer_id, "peer-456");
            assert_eq!(my_name, "MyPC");
            assert_eq!(tcp_port, 8080);
        }
        _ => panic!("Expected DiscoveryRequest variant"),
    }
}

#[test]
fn test_discovery_response_serialize() {
    let msg = DiscoveryMsg::DiscoveryResponse {
        peer_id: "responder-789".to_string(),
        my_name: "ResponderPC".to_string(),
        tcp_port: 9001,
    };

    let json = serde_json::to_string(&msg).expect("Should serialize");
    assert!(json.contains("DiscoveryResponse"));
    assert!(json.contains("responder-789"));
    assert!(json.contains("ResponderPC"));
}

#[test]
fn test_discovery_response_deserialize() {
    let json = r#"{"DiscoveryResponse":{"peer_id":"resp-id","my_name":"RespPC","tcp_port":7070}}"#;
    let msg: DiscoveryMsg = serde_json::from_str(json).expect("Should deserialize");

    match msg {
        DiscoveryMsg::DiscoveryResponse {
            peer_id,
            my_name,
            tcp_port,
        } => {
            assert_eq!(peer_id, "resp-id");
            assert_eq!(my_name, "RespPC");
            assert_eq!(tcp_port, 7070);
        }
        _ => panic!("Expected DiscoveryResponse variant"),
    }
}

#[test]
fn test_discovery_msg_roundtrip() {
    let original = DiscoveryMsg::DiscoveryRequest {
        peer_id: "roundtrip-id".to_string(),
        my_name: "RoundtripPC".to_string(),
        tcp_port: 5555,
    };

    // Serialize to bytes (like in real usage)
    let bytes = serde_json::to_vec(&original).expect("Should serialize to vec");

    // Deserialize back
    let deserialized: DiscoveryMsg =
        serde_json::from_slice(&bytes).expect("Should deserialize from slice");

    match deserialized {
        DiscoveryMsg::DiscoveryRequest {
            peer_id,
            my_name,
            tcp_port,
        } => {
            assert_eq!(peer_id, "roundtrip-id");
            assert_eq!(my_name, "RoundtripPC");
            assert_eq!(tcp_port, 5555);
        }
        _ => panic!("Expected DiscoveryRequest variant"),
    }
}
