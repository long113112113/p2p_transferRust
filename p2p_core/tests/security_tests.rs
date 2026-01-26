use futures_util::{SinkExt, StreamExt};
use p2p_core::AppEvent;

#[tokio::test]
async fn test_websocket_upload_overflow() {
    // Setup
    let token = "test_overflow";
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let upload_state = std::sync::Arc::new(p2p_core::http_share::websocket::UploadState::default());

    // Use temp dir
    let temp_dir = std::env::temp_dir().join(format!("p2p_test_{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let download_dir = temp_dir.clone();

    // Create router
    let router = p2p_core::http_share::server::create_router_with_websocket(
        token,
        tx.clone(),
        upload_state.clone(),
        download_dir,
    );

    // Start server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // Give server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Connect client
    let ws_url = format!("ws://127.0.0.1:{}/{}/ws", port, token);
    let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    let (mut write, mut read) = ws_stream.split();

    // 1. Send FileInfo (size = 100)
    let msg = p2p_core::http_share::websocket::ClientMessage::FileInfo {
        file_name: "overflow.txt".to_string(),
        file_size: 100,
    };
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(&msg).unwrap().into(),
        ))
        .await
        .unwrap();

    // 2. Wait for UploadRequest event and accept it
    let request_id = tokio::time::timeout(tokio::time::Duration::from_secs(2), async {
        while let Some(evt) = rx.recv().await {
            if let AppEvent::UploadRequest { request_id, .. } = evt {
                return Some(request_id);
            }
        }
        None
    })
    .await
    .expect("Timeout waiting for request")
    .expect("No request received");

    // Accept upload
    p2p_core::http_share::websocket::respond_to_upload(&upload_state, &request_id, true).await;

    // 3. Wait for Accepted message from WebSocket
    let mut accepted = false;
    while let Some(msg) = read.next().await {
        if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
            if text.contains("accepted") {
                accepted = true;
                break;
            }
        }
    }
    assert!(accepted, "Should have received accepted message");

    // 4. Send more data than declared (1024 bytes vs 100 declared)
    let data = vec![0u8; 1024];
    write
        .send(tokio_tungstenite::tungstenite::Message::Binary(data.into()))
        .await
        .unwrap();

    // 5. Wait for Complete message or Error
    // We expect the transfer to "complete" because the server will see received_bytes >= file_size
    // But we are interested in what ends up on disk.

    let mut completed = false;
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(2), async {
        while let Some(msg) = read.next().await {
            if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = msg {
                if text.contains("complete") {
                    completed = true;
                    break;
                }
            }
        }
    })
    .await;

    assert!(completed, "Upload should complete");

    // 6. Check file size
    let file_path = temp_dir.join("overflow.txt");
    let metadata = tokio::fs::metadata(&file_path).await.unwrap();

    // Clean up
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;

    // Assert strictly
    assert_eq!(
        metadata.len(),
        100,
        "File size {} should match declared size 100",
        metadata.len()
    );
}
