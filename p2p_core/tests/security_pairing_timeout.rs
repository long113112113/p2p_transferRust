use p2p_core::transfer::{make_client_endpoint, make_server_endpoint, run_server};
use p2p_core::transfer::protocol::{recv_msg, send_msg, TransferMsg};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_pairing_dos_reproduction() {
    // Set timeout to 2 seconds for this test
    unsafe { std::env::set_var("P2P_PAIRING_TIMEOUT", "2"); }

    // Install crypto provider if needed
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 1. Setup Server
    let server_endpoint = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();
    let (tx, _rx) = mpsc::channel(100);
    let download_dir = PathBuf::from("downloads");

    // Spawn server
    let endpoint_clone = server_endpoint.clone();
    tokio::spawn(async move {
        run_server(endpoint_clone, tx, download_dir).await;
    });

    // 2. Spawn 3 "Attacker" Clients that hang during handshake
    let mut attacker_handles = Vec::new();
    let client_endpoint = make_client_endpoint().unwrap();

    for i in 0..3 {
        let endpoint = client_endpoint.clone();
        let handle = tokio::spawn(async move {
            let connection = endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
            let (mut send, mut recv) = connection.open_bi().await.unwrap();

            // Send PairingRequest
            send_msg(
                &mut send,
                &TransferMsg::PairingRequest {
                    endpoint_id: format!("attacker-{}", i),
                    peer_name: format!("Attacker {}", i),
                },
            )
            .await
            .unwrap();

            // Receive VerificationRequired
            let msg = recv_msg(&mut recv).await.unwrap();
            match msg {
                TransferMsg::VerificationRequired => {
                    // Correct behavior: User sees code, enters it, sends VerificationCode.
                    // MALICIOUS behavior: Do nothing, hang indefinitely.
                    // We just sleep to hold the connection open.
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
                _ => panic!("Expected VerificationRequired, got {:?}", msg),
            }
        });
        attacker_handles.push(handle);
    }

    // Give attackers time to connect and consume slots
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 3. Try to connect a 4th "Legitimate" Client
    let legitimate_endpoint = client_endpoint.clone();
    let connection = legitimate_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();

    // Send PairingRequest
    send_msg(
        &mut send,
        &TransferMsg::PairingRequest {
            endpoint_id: "legitimate-user".to_string(),
            peer_name: "Legitimate User".to_string(),
        },
    )
    .await
    .unwrap();

    // Expect Rejection (VerificationFailed) immediately
    // Because slots are full (3 attackers holding slots)
    let msg = recv_msg(&mut recv).await.unwrap();

    match msg {
        TransferMsg::VerificationFailed { message } => {
            assert!(message.contains("Too many pending attempts") || message.contains("Too many pending verification attempts"), "Expected 'Too many pending attempts', got: {}", message);
        }
        _ => panic!("Expected VerificationFailed due to DoS, got {:?}", msg),
    }

    // Cleanup attackers
    // (Test will end and drop handles)

    // 4. Verify that after timeout (2s), the slots are freed
    println!("Waiting for timeout...");
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    println!("Connecting 5th client...");
    let success_endpoint = client_endpoint.clone();
    let connection = success_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();

    send_msg(
        &mut send,
        &TransferMsg::PairingRequest {
            endpoint_id: "success-user".to_string(),
            peer_name: "Success User".to_string(),
        },
    )
    .await
    .unwrap();

    // Should receive VerificationRequired now
    let msg = recv_msg(&mut recv).await.unwrap();
    match msg {
        TransferMsg::VerificationRequired => {
            // Success!
        }
        _ => panic!("Expected VerificationRequired after timeout, got {:?}", msg),
    }
}
