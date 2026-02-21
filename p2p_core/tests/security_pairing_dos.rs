use p2p_core::transfer::protocol::TransferMsg;
use p2p_core::transfer::server::run_server;
use p2p_core::transfer::{make_client_endpoint, make_server_endpoint};
use tokio::sync::mpsc;

#[tokio::test]
async fn test_pairing_dos_fix() {
    // Set short timeout for testing
    // SAFETY: This is a test environment.
    unsafe {
        std::env::set_var("P2P_PAIRING_TIMEOUT", "1");
    }

    // Install crypto provider
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 1. Setup Server
    let server_endpoint = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();

    let (event_tx, _event_rx) = mpsc::channel(100);
    let download_dir = std::env::temp_dir().join(format!("p2p_dos_fix_test_{}", uuid::Uuid::new_v4()));
    let _ = tokio::fs::create_dir_all(&download_dir).await;

    // Spawn server
    let _server_handle = tokio::spawn(async move {
        run_server(server_endpoint, event_tx, download_dir).await;
    });

    // 2. Occupy all pairing slots (3)
    let mut malicious_clients = Vec::new();
    for i in 0..3 {
        let client_endpoint = make_client_endpoint().unwrap();
        let connection = client_endpoint
            .connect(server_addr, "localhost")
            .unwrap()
            .await
            .expect("Failed to connect malicious client");

        let (mut send, mut recv) = connection.open_bi().await.expect("Failed to open stream");

        // Send PairingRequest
        let msg = TransferMsg::PairingRequest {
            endpoint_id: format!("malicious_{}", i),
            peer_name: "Malicious".to_string(),
        };
        let json = serde_json::to_vec(&msg).unwrap();
        let len = (json.len() as u32).to_be_bytes();

        {
            use tokio::io::AsyncWriteExt;
            send.write_all(&len).await.unwrap();
            send.write_all(&json).await.unwrap();
        }

        // Read VerificationRequired
        let mut len_buf = [0u8; 4];
        {
            use tokio::io::AsyncReadExt;
            recv.read_exact(&mut len_buf).await.unwrap();
            let len = u32::from_be_bytes(len_buf) as usize;
            let mut buf = vec![0u8; len];
            recv.read_exact(&mut buf).await.unwrap();
            let response: TransferMsg = serde_json::from_slice(&buf).unwrap();

            if let TransferMsg::VerificationRequired = response {
                // Good, we are now holding a slot.
                // DO NOT send the verification code.
            } else {
                panic!("Expected VerificationRequired, got {:?}", response);
            }
        }

        malicious_clients.push((connection, send, recv));
    }

    // 3. Wait for timeout (1s + buffer)
    tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

    // 4. Try to connect a 4th client - should SUCCEED now because slots are freed
    let client_endpoint = make_client_endpoint().unwrap();
    let connection = client_endpoint
        .connect(server_addr, "localhost")
        .unwrap()
        .await
        .expect("Failed to connect legitimate client");

    let (mut send, mut recv) = connection.open_bi().await.expect("Failed to open stream");

    let msg = TransferMsg::PairingRequest {
        endpoint_id: "legitimate".to_string(),
        peer_name: "Legitimate".to_string(),
    };
    let json = serde_json::to_vec(&msg).unwrap();
    let len = (json.len() as u32).to_be_bytes();
    {
        use tokio::io::AsyncWriteExt;
        send.write_all(&len).await.unwrap();
        send.write_all(&json).await.unwrap();
    }

    // Read response
    let mut len_buf = [0u8; 4];
    {
        use tokio::io::AsyncReadExt;
        recv.read_exact(&mut len_buf).await.unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        recv.read_exact(&mut buf).await.unwrap();
        let response: TransferMsg = serde_json::from_slice(&buf).unwrap();

        match response {
            TransferMsg::VerificationRequired => {
                // Success! We got a slot.
            }
            TransferMsg::VerificationFailed { message } => {
                panic!("Still failing with: {}", message);
            }
            _ => panic!("Expected VerificationRequired, got {:?}", response),
        }
    }

    // Cleanup
    for (conn, _, _) in malicious_clients {
        conn.close(0u32.into(), &[]);
    }
}
