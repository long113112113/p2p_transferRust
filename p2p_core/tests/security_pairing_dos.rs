use p2p_core::transfer::{make_server_endpoint, make_client_endpoint, protocol::{TransferMsg, send_msg, recv_msg}};
use tokio::sync::mpsc;
use std::time::Duration;

#[tokio::test]
async fn test_pairing_dos_timeout() {
    // Install crypto provider
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Set timeout to 1 second for test
    // SAFETY: This is a test, and we are setting the environment variable before spawning the server.
    unsafe { std::env::set_var("P2P_PAIRING_TIMEOUT", "1"); }

    // 1. Setup Server
    let (tx, mut rx) = mpsc::channel(100);
    // Use a unique directory for this test
    let download_dir = std::env::temp_dir().join(format!("p2p_test_dos_{}", uuid::Uuid::new_v4()));
    let _ = tokio::fs::create_dir_all(&download_dir).await;

    let server_endpoint = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();

    let server_endpoint_clone = server_endpoint.clone();
    tokio::spawn(async move {
        p2p_core::transfer::run_server(server_endpoint_clone, tx, download_dir).await;
    });

    // Spawn a task to drain the event channel so the server doesn't block on sending events
    tokio::spawn(async move {
        while let Some(_) = rx.recv().await {}
    });

    // 2. Connect 3 stalling clients (MAX_PAIRING_ATTEMPTS = 3)
    let client_endpoint = make_client_endpoint().unwrap();
    let mut stalled_conns = Vec::new();

    for i in 0..3 {
        let connection = client_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();

        // Send PairingRequest
        let msg = TransferMsg::PairingRequest {
            endpoint_id: format!("attacker_{}", i),
            peer_name: "Attacker".to_string(),
        };
        send_msg(&mut send, &msg).await.unwrap();

        // Read VerificationRequired
        let resp = recv_msg(&mut recv).await.unwrap();
        if let TransferMsg::VerificationRequired = resp {
            // Good, now STALL. Do not send code.
            stalled_conns.push((send, recv));
        } else {
            panic!("Expected VerificationRequired, got {:?}", resp);
        }
    }

    // 3. Try 4th client - should be rejected immediately (slots full)
    {
        let connection = client_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();

        let msg = TransferMsg::PairingRequest {
            endpoint_id: "victim_1".to_string(),
            peer_name: "Victim 1".to_string(),
        };
        send_msg(&mut send, &msg).await.unwrap();

        let resp = recv_msg(&mut recv).await.unwrap();
        match resp {
            TransferMsg::VerificationFailed { message } => {
                assert_eq!(message, "Too many pending verification attempts");
            },
            _ => panic!("Expected VerificationFailed immediately, got {:?}", resp),
        }
    }

    // 4. Wait for timeout (1.5s > 1s)
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // 5. Try 5th client - should SUCCEED if timeout works (slots freed)
    // If vulnerability exists (no timeout), this will fail with "Too many pending verification attempts"
    {
        let connection = client_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();

        let msg = TransferMsg::PairingRequest {
            endpoint_id: "victim_2".to_string(),
            peer_name: "Victim 2".to_string(),
        };
        send_msg(&mut send, &msg).await.unwrap();

        let resp = recv_msg(&mut recv).await.unwrap();
        match resp {
            TransferMsg::VerificationRequired => {
                // Success! Slots were freed.
            },
            TransferMsg::VerificationFailed { message } => {
                // Failure! Slots still occupied.
                panic!("Vulnerability confirmed: Client rejected after timeout wait: {}", message);
            },
            _ => panic!("Unexpected response: {:?}", resp),
        }
    }
}
