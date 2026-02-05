use p2p_core::transfer::{make_server_endpoint, make_client_endpoint, protocol::recv_msg};
use p2p_core::transfer::constants::MAX_MSG_SIZE;

#[tokio::test]
async fn test_large_message_rejection() {
    // Install crypto provider if needed (might fail if already installed, which is fine)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 1. Setup Server
    let server_endpoint = make_server_endpoint("127.0.0.1:0".parse().unwrap()).unwrap();
    let server_addr = server_endpoint.local_addr().unwrap();

    // 2. Setup Client
    let client_endpoint = make_client_endpoint().unwrap();

    // 3. Spawn Server Logic
    let server_handle = tokio::spawn(async move {
        if let Some(incoming) = server_endpoint.accept().await {
            let connection = incoming.await.unwrap();
            let (mut _send, mut recv) = connection.accept_bi().await.unwrap();

            // Try to receive message - expecting failure
            let result = recv_msg(&mut recv).await;

            assert!(result.is_err(), "Should fail to receive large message");
            let err = result.unwrap_err();
            assert!(err.to_string().contains("Message too large"), "Error should be about message size: {}", err);
        }
    });

    // 4. Connect Client and Send Malicious Message
    let connection = client_endpoint.connect(server_addr, "localhost").unwrap().await.unwrap();
    let (mut send, mut _recv) = connection.open_bi().await.unwrap();

    // 5. Send Large Length Prefix
    // We want to send a length that is > MAX_MSG_SIZE (64KB)
    let bad_len: u32 = (MAX_MSG_SIZE + 1000) as u32;
    send.write_all(&bad_len.to_be_bytes()).await.unwrap();

    // We don't need to send the actual body, the server should fail immediately after reading length.
    // But we send a few bytes just in case it tries to read.
    let _ = send.write_all(&[0u8; 10]).await;

    // finish() is synchronous in recent quinn versions or returns Result
    let _ = send.finish();

    // 6. Wait for server result
    // Use timeout to prevent hanging if test fails
    if let Err(_) = tokio::time::timeout(tokio::time::Duration::from_secs(2), server_handle).await {
        panic!("Test timed out");
    }
}
