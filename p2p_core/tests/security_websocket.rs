#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use p2p_core::http_share::server::create_router_with_websocket;
    use p2p_core::http_share::websocket::{ClientMessage, ServerMessage, UploadState, respond_to_upload};
    use p2p_core::AppEvent;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::mpsc;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    #[tokio::test]
    async fn test_write_limit_enforcement() {
        // Setup
        let token = "test_token_limit_enforce";
        let (tx, mut rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::default());
        let download_dir = std::env::temp_dir().join("p2p_test_limit");
        let _ = tokio::fs::create_dir_all(&download_dir).await;

        // Clean up previous run
        let target_file = download_dir.join("oversized.txt");
        let _ = tokio::fs::remove_file(&target_file).await;

        let router = create_router_with_websocket(token, tx, upload_state.clone(), download_dir.clone());

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

        // 1. Send FileInfo with small size
        let claimed_size = 10;
        let msg = ClientMessage::FileInfo {
            file_name: "oversized.txt".to_string(),
            file_size: claimed_size,
        };
        write
            .send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
            .await
            .unwrap();

        // 2. Wait for UploadRequest event and extract ID
        let request_id = tokio::time::timeout(tokio::time::Duration::from_secs(2), async {
            while let Some(evt) = rx.recv().await {
                if let AppEvent::UploadRequest { request_id, .. } = evt {
                    return Some(request_id);
                }
            }
            None
        })
        .await
        .expect("Timeout waiting for UploadRequest")
        .expect("Did not receive UploadRequest");

        // 3. Approve request
        respond_to_upload(&upload_state, &request_id, true).await;

        // 4. Consume "Accepted" message from WS
        loop {
             match read.next().await {
                 Some(Ok(Message::Text(text))) => {
                     let msg: ServerMessage = serde_json::from_str(&text).unwrap();
                     if let ServerMessage::Accepted { .. } = msg {
                         break;
                     }
                 }
                 _ => continue,
             }
        }

        // 5. Send MORE data than claimed (20 bytes vs 10 bytes claimed)
        let data = vec![b'A'; 20];
        write.send(Message::Binary(data.into())).await.unwrap();

        // 6. Wait for completion or error
        // The server might close connection or send Complete
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // 7. Check file size on disk
        let metadata = tokio::fs::metadata(&target_file).await.expect("File not found");

        // FAIL IF size > claimed_size
        // With current bug, this assertion will likely fail if we strictly check equality,
        // or pass if we check <= but the bug allows >.
        // We want to prove it IS > claimed_size currently.

        println!("File size on disk: {}", metadata.len());
        assert!(metadata.len() <= claimed_size, "File on disk ({}) is larger than claimed size ({})", metadata.len(), claimed_size);

        // Cleanup
        let _ = tokio::fs::remove_file(&target_file).await;
        let _ = tokio::fs::remove_dir(&download_dir).await;
    }
}
