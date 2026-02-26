use p2p_core::transfer::protocol::{TransferMsg, recv_msg, send_msg};
use p2p_core::transfer::{make_client_endpoint, make_server_endpoint, run_server};
use tokio::sync::mpsc;
use tokio::time::{Duration, sleep};

#[tokio::test]
async fn test_pairing_dos_vulnerability() {
    // Install crypto provider if needed (might fail if already installed, which is fine)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Set a short timeout for the fix (2 seconds)
    // For reproduction (before fix), this won't matter as there is no timeout
    unsafe {
        std::env::set_var("P2P_PAIRING_TIMEOUT", "2");
    }

    // 1. Setup Server
    let server_addr = "127.0.0.1:0".parse().unwrap();
    let server_endpoint = make_server_endpoint(server_addr).unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();

    let (event_tx, _event_rx) = mpsc::channel(100);
    let download_dir = std::env::temp_dir().join("p2p_dos_test");
    let _ = tokio::fs::create_dir_all(&download_dir).await;

    // Spawn server
    let server_handle = tokio::spawn(async move {
        run_server(server_endpoint, event_tx, download_dir).await;
    });

    // 2. Setup Client Endpoint (shared)
    let client_endpoint = make_client_endpoint().unwrap();

    // 3. Connect 3 "Attackers"
    let mut attacker_conns = Vec::new();

    for i in 0..3 {
        let endpoint = client_endpoint.clone();
        let conn = endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .expect("Attacker failed to connect");

        let (mut send, mut recv) = conn.open_bi().await.expect("Attacker failed to open stream");

        // Send PairingRequest
        let msg = TransferMsg::PairingRequest {
            endpoint_id: format!("attacker_{}", i),
            peer_name: format!("Attacker {}", i),
        };
        send_msg(&mut send, &msg).await.expect("Attacker failed to send request");

        // Wait for VerificationRequired
        let response = recv_msg(&mut recv).await.expect("Attacker failed to receive response");
        if let TransferMsg::VerificationRequired = response {
            // Good, we are holding a slot
        } else {
            panic!("Attacker {} got unexpected response: {:?}", i, response);
        }

        // STALL: Do not send code. Keep connection open.
        attacker_conns.push((conn, send, recv));
    }

    // 4. Connect "Victim" - Should be rejected immediately (DoS)
    {
        let endpoint = client_endpoint.clone();
        let conn = endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .expect("Victim failed to connect");

        let (mut send, mut recv) = conn.open_bi().await.expect("Victim failed to open stream");

        // Send PairingRequest
        let msg = TransferMsg::PairingRequest {
            endpoint_id: "victim".to_string(),
            peer_name: "Victim".to_string(),
        };
        send_msg(&mut send, &msg).await.expect("Victim failed to send request");

        // Expect Failure
        let response = recv_msg(&mut recv).await.expect("Victim failed to receive response");
        match response {
            TransferMsg::VerificationFailed { message } => {
                assert!(message.contains("Too many pending"), "Expected 'Too many pending', got: {}", message);
                println!("Confirmed DoS: Victim rejected as expected.");
            }
            _ => panic!("Victim succeeded but should have failed! DoS not active? Got: {:?}", response),
        }
    }

    // 5. Wait for Timeout (3 seconds)
    // If the fix is working, the server should drop the attackers after 2 seconds (P2P_PAIRING_TIMEOUT)
    println!("Waiting for timeout (3s)...");
    sleep(Duration::from_secs(3)).await;

    // 6. Connect "New Victim" - Should SUCCEED if fix works
    {
        let endpoint = client_endpoint.clone();
        let conn = endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .expect("New Victim failed to connect");

        let (mut send, mut recv) = conn.open_bi().await.expect("New Victim failed to open stream");

        // Send PairingRequest
        let msg = TransferMsg::PairingRequest {
            endpoint_id: "victim_new".to_string(),
            peer_name: "New Victim".to_string(),
        };
        send_msg(&mut send, &msg).await.expect("New Victim failed to send request");

        // Expect VerificationRequired (Success)
        // Note: recv_msg might fail if server closed connection (if fix is NOT implemented yet, or if attackers still hold slots)
        // If fix is NOT implemented, attackers still hold slots, so this should FAIL.
        // If fix IS implemented, attackers were dropped, so this should SUCCEED.

        // Since we are running this test BEFORE the fix to confirm vulnerability,
        // we expect this to FAIL (or timeout waiting if we didn't implement timeout yet).

        // Wait, "Wait for timeout" step relies on the fix being present.
        // Without the fix, the attackers will hold the slots forever.
        // So this second attempt should ALSO fail if the bug is present.

        let result = recv_msg(&mut recv).await;

        // Check if we are running with or without fix?
        // Actually, this test is designed to PASS only after the fix.
        // Before the fix, it should FAIL at this assertion.
        match result {
             Ok(TransferMsg::VerificationRequired) => {
                 println!("Success: Slots were freed!");
             },
             Ok(TransferMsg::VerificationFailed { message }) => {
                 panic!("Failed: Slots still blocked after timeout! Message: {}", message);
             }
             Err(e) => {
                 panic!("Failed to receive response: {}", e);
             }
             _ => panic!("Unexpected response"),
        }
    }

    // Cleanup
    server_handle.abort();
}
