use p2p_core::transfer::{make_client_endpoint, make_server_endpoint, run_server};
use std::path::PathBuf;
use tokio::sync::mpsc;

#[tokio::test]
async fn test_initial_connection_hang() {
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

    // 2. Connect client and HANG
    let client_endpoint = make_client_endpoint().unwrap();
    let connection = client_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
    let (mut send, mut recv) = connection.open_bi().await.unwrap();

    // Send 1 byte to ensure server accepts the stream
    send.write_all(&[0]).await.unwrap();

    // Send nothing else.
    // If server has a timeout on initial message, it should close the stream.
    // We expect the server to close the stream within a reasonable time (e.g., 5-10s).

    // We wait for 6 seconds (server timeout is 5s).
    // If the read returns, the server closed the stream (GOOD).
    // If it timeouts, the server is still waiting (BAD).

    let mut buf = [0u8; 10];
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(6), recv.read(&mut buf)).await;

    match result {
        Ok(_) => {
            // Server closed stream: success
        }
        Err(_) => {
            panic!("Server did not close the stream within 6 seconds.");
        }
    }
}
