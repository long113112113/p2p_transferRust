use p2p_core::transfer::{make_server_endpoint, make_client_endpoint, protocol::{send_msg, TransferMsg}, run_server};
use p2p_core::FileInfo;
use tokio::sync::mpsc;
use std::path::PathBuf;

#[tokio::test]
async fn test_unauthenticated_upload_prevention() {
    // Install crypto provider if needed (might fail if already installed, which is fine)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 1. Setup Server
    let server_endpoint = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();

    let (event_tx, _event_rx) = mpsc::channel(100);
    let temp_dir = std::env::temp_dir().join(format!("p2p_test_unauth_{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let download_dir = temp_dir.clone();

    // Run server in background
    let server_handle = tokio::spawn(async move {
        run_server(server_endpoint, event_tx, download_dir).await;
    });

    // 2. Setup Client
    let client_endpoint = make_client_endpoint().unwrap();
    // Connect
    let connection = client_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();

    // 3. Connect and send FileMetadata WITHOUT handshake (PairingRequest)
    // This simulates an attacker bypassing the auth flow
    let (mut send, mut _recv) = connection.open_bi().await.unwrap();

    let file_info = FileInfo {
        file_name: "hacked.txt".to_string(),
        file_size: 10,
        file_path: PathBuf::new(),
        file_hash: None,
    };

    // Send metadata directly
    send_msg(&mut send, &TransferMsg::FileMetadata { info: file_info }).await.unwrap();

    // Send data immediately
    send.write_all(b"hackdetected").await.unwrap();
    // Finish stream
    let _ = send.finish();

    // 4. Verify Vulnerability
    // Allow server time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let file_path = temp_dir.join("hacked.txt");
    let file_exists = file_path.exists();

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    server_handle.abort(); // Stop server

    // Assert that the file was NOT created (this will fail before the fix)
    assert!(!file_exists, "Unauthenticated file upload succeeded! Vulnerability confirmed.");
}
