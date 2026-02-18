use p2p_core::http_share::server::create_router_with_websocket;
use p2p_core::http_share::websocket::{UploadState, ClientMessage};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use std::path::PathBuf;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use std::net::SocketAddr;

#[tokio::test]
async fn test_large_message_dos() {
    // Setup
    let token = "test_token_dos";
    let (tx, _rx) = mpsc::channel(100);
    let upload_state = Arc::new(UploadState::default());
    let download_dir = PathBuf::from(".");
    let router = create_router_with_websocket(token, tx, upload_state, download_dir);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // Spawn server
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // Give server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let ws_url = format!("ws://127.0.0.1:{}/{}/ws", port, token);

    // Connect client
    let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
    let (mut write, mut read) = ws_stream.split();

    // Create a large text message (e.g., 5MB)
    let large_text = " ".repeat(5 * 1024 * 1024);

    // Send might succeed because it's async and buffered
    if let Err(e) = write.send(Message::Text(large_text.into())).await {
        println!("Send failed (expected if limit exists): {}", e);
        return;
    }

    // Wait for response
    // If limit is enforced, we expect connection closure or error
    let result = tokio::time::timeout(tokio::time::Duration::from_secs(2), read.next()).await;

    match result {
        Ok(Some(Ok(msg))) => {
             if let Message::Close(frame) = msg {
                 println!("Connection closed with frame: {:?}", frame);
                 // 1009 is Message Too Big
                 if let Some(f) = frame {
                     assert_eq!(u16::from(f.code), 1009, "Should be closed with code 1009 (Message Too Big)");
                 }
             } else if let Message::Text(text) = msg {
                 if text.contains("Expected file_info message") {
                     panic!("Server accepted large message! Vulnerability present.");
                 }
             }
        }
        Ok(Some(Err(e))) => {
            println!("Connection error as expected: {}", e);
        }
        Ok(None) => {
            println!("Connection closed as expected");
        }
        Err(_) => {
             // Timeout might happen if server is slow processing huge message
             panic!("Timeout! Server accepted large message (likely).");
        }
    }
}

#[tokio::test]
async fn test_normal_message_size() {
    // Setup
    let token = "test_token_normal";
    let (tx, _rx) = mpsc::channel(100);
    let upload_state = Arc::new(UploadState::default());
    let download_dir = PathBuf::from(".");
    let router = create_router_with_websocket(token, tx, upload_state, download_dir);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    let ws_url = format!("ws://127.0.0.1:{}/{}/ws", port, token);

    let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
    let (mut write, mut read) = ws_stream.split();

    // Send 256KB text message (invalid protocol but valid size)
    // 256KB + slightly more to match CHUNK_SIZE exactly if needed, but text size is exact chars
    let text = " ".repeat(256 * 1024);
    write.send(Message::Text(text.into())).await.unwrap();

    // Expect: Not "Message Too Big".
    // Probably "Error: Expected file_info message" (because it fails JSON parse) or "Error: Invalid JSON"
    // But definitely NOT connection closure with code 1009.

    let result = tokio::time::timeout(tokio::time::Duration::from_secs(2), read.next()).await;

    match result {
        Ok(Some(Ok(msg))) => {
             if let Message::Close(frame) = msg {
                 if let Some(f) = frame {
                     if u16::from(f.code) == 1009 {
                         panic!("Connection closed with Message Too Big for 256KB message! Limit is too low.");
                     }
                 }
             }
             // Any other message (Text error, etc.) means it was accepted by WS layer.
        }
        Ok(Some(Err(_))) => {}
        Ok(None) => {}
        Err(_) => {}
    }
}
