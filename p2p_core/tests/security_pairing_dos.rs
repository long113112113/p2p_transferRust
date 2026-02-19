use p2p_core::transfer::{make_client_endpoint, make_server_endpoint, protocol::recv_msg};
use p2p_core::transfer::protocol::TransferMsg;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn test_pairing_handshake_dos() {
    // Install crypto provider if needed
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Set a short timeout for the test via environment variable
    // This will be used by the server implementation
    unsafe {
        std::env::set_var("P2P_PAIRING_TIMEOUT", "1");
    }

    // 1. Setup Server
    // Use port 0 to let OS assign a random port
    let server_endpoint = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(100);

    // Use a temp dir for download path
    let temp_dir = std::env::temp_dir().join(format!("p2p_test_dos_{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();

    // Spawn Server Logic
    let endpoint_clone = server_endpoint.clone();
    let dir_clone = temp_dir.clone();
    tokio::spawn(async move {
        p2p_core::transfer::run_server(endpoint_clone, tx, dir_clone).await;
    });

    // 2. Setup Client
    let client_endpoint = make_client_endpoint().unwrap();

    // 3. Connect MAX_PAIRING_ATTEMPTS (3) clients and stall them
    let stalled_clients = Arc::new(AtomicUsize::new(0));

    for i in 0..3 {
        let client = client_endpoint.clone();
        let stalled_counter = stalled_clients.clone();

        tokio::spawn(async move {
            let connection = client.connect(server_addr, "localhost").unwrap().await.unwrap();
            let (mut send, mut recv) = connection.open_bi().await.unwrap();

            // Send PairingRequest
            let msg = TransferMsg::PairingRequest {
                endpoint_id: format!("attacker_{}", i),
                peer_name: format!("Attacker {}", i),
            };

            // Serialize and send manually or use send_msg if accessible (it's not public in tests easily,
            // but we can use the protocol logic or just copy send_msg logic)
            // Actually, send_msg is not re-exported for tests in lib.rs?
            // `pub use server::run_server` is there, but `send_msg` is in protocol.
            // Let's implement a simple send helper here or assume we can access it via p2p_core::transfer::protocol::send_msg
            // if it was public. But checking mod.rs, protocol is pub mod but send_msg is pub async fn.
            // So p2p_core::transfer::protocol::send_msg should be accessible.

            p2p_core::transfer::protocol::send_msg(&mut send, &msg).await.unwrap();

            // Receive VerificationRequired
            let response = recv_msg(&mut recv).await.unwrap();
            if let TransferMsg::VerificationRequired = response {
                stalled_counter.fetch_add(1, Ordering::SeqCst);
                // NOW STALL: Do NOT send VerificationCode
                // Just keep connection open
                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
            }
        });
    }

    // Wait for all 3 clients to reach the stalled state
    // We give them a bit of time
    let start = std::time::Instant::now();
    while stalled_clients.load(Ordering::SeqCst) < 3 {
        if start.elapsed().as_secs() > 5 {
            panic!("Failed to spawn 3 stalled clients");
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // 4. Try 4th connection - Should be REJECTED immediately
    {
        let connection = client_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();

        let msg = TransferMsg::PairingRequest {
            endpoint_id: "victim".to_string(),
            peer_name: "Victim".to_string(),
        };
        p2p_core::transfer::protocol::send_msg(&mut send, &msg).await.unwrap();

        let response = recv_msg(&mut recv).await.unwrap();

        match response {
            TransferMsg::VerificationFailed { message } => {
                assert_eq!(message, "Too many pending verification attempts");
            }
            _ => panic!("Expected VerificationFailed, got {:?}", response),
        }
    }

    // 5. Wait for the timeout (we set it to 1s, so wait 2s to be safe)
    // The server should drop the stalled connections.
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    // 6. Try 4th connection AGAIN - Should SUCCEED (or proceed to VerificationRequired)
    // If the vulnerability exists (no timeout), the slots are still held, so this will fail.
    // If fixed, this will proceed.
    {
        let connection = client_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
        let (mut send, mut recv) = connection.open_bi().await.unwrap();

        let msg = TransferMsg::PairingRequest {
            endpoint_id: "victim_retry".to_string(),
            peer_name: "Victim Retry".to_string(),
        };
        p2p_core::transfer::protocol::send_msg(&mut send, &msg).await.unwrap();

        let response = recv_msg(&mut recv).await.unwrap();

        match response {
            TransferMsg::VerificationRequired => {
                // Success! We got a slot.
            }
            TransferMsg::VerificationFailed { message } => {
                 if message == "Too many pending verification attempts" {
                     panic!("VULNERABILITY CONFIRMED: Server did not free up slots after timeout!");
                 } else {
                     panic!("Unexpected failure: {}", message);
                 }
            }
            _ => panic!("Expected VerificationRequired, got {:?}", response),
        }
    }

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}
