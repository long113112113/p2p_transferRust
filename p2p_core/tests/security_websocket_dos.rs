use p2p_core::http_share::server::create_router_with_websocket;
use p2p_core::http_share::websocket::{UploadState, ClientMessage};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn test_websocket_message_size_limit() {
    // Setup server
    let token = "test_token_limit";
    let (tx, _rx) = mpsc::channel(100);
    let upload_state = Arc::new(UploadState::default());
    let download_dir = std::env::temp_dir().join("p2p_test_downloads");
    tokio::fs::create_dir_all(&download_dir).await.unwrap();

    // Create router
    let router = create_router_with_websocket(token, tx, upload_state, download_dir);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // Spawn server
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
    });

    // Wait for server start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let ws_url = format!("ws://127.0.0.1:{}/{}/ws", port, token);

    // Connect client
    let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
    let (mut write, mut read) = ws_stream.split();

    // Send FileInfo to start handshake (otherwise we might get rejected early)
    let msg = ClientMessage::FileInfo {
        file_name: "large_file.bin".to_string(),
        file_size: 10 * 1024 * 1024, // 10MB
    };
    write.send(Message::Text(serde_json::to_string(&msg).unwrap().into())).await.unwrap();

    // Wait for response (Accepted or Error)
    // In a real scenario we'd wait for approval, but for this test we just want to see if the connection stays open
    // when we send a large message.

    // Construct a large message (1MB) - larger than our target limit (512KB) but smaller than default (64MB)
    let large_data = vec![0u8; 1024 * 1024]; // 1MB

    // Send the large message
    let result = write.send(Message::Binary(large_data.into())).await;

    if let Err(e) = result {
        // If we can't even send it, the server might have closed the connection already (which is good if configured, bad if not)
        // But tungstenite client doesn't enforce limit on send by default.
        println!("Send error: {}", e);
    }

    // Check if server closes connection with PolicyViolation or similar
    // We expect the server to close the connection if the limit is enforced.
    // If not enforced, it will try to process it (or at least keep connection open).

    let next_msg = tokio::time::timeout(tokio::time::Duration::from_secs(2), read.next()).await;

    match next_msg {
        Ok(Some(Ok(Message::Close(Some(frame))))) => {
             // 1009 is Message Too Big
            if frame.code == tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Size {
                println!("Connection closed with Size limit violation (Good!)");
            } else {
                println!("Connection closed with code: {}", frame.code);
            }
        },
        Ok(Some(Ok(_))) => {
            println!("Received message (Server accepted it or sent something else)");
            // If we receive something, it means the connection is still alive and the message was likely buffered.
            // This indicates vulnerability if we expected it to be rejected.
             panic!("VULNERABILITY: Server accepted 1MB message!");
        },
        Ok(None) => {
             println!("Stream ended (Connection closed without close frame)");
        },
        Ok(Some(Err(e))) => {
             println!("Error reading: {}", e);
        },
        Err(_) => {
            println!("Timeout waiting for response");
             panic!("VULNERABILITY: Server kept connection open for 1MB message!");
        }
    }
}
