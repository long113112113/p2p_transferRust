#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use p2p_core::AppEvent;
    use p2p_core::http_share::server::create_router_with_websocket;
    use p2p_core::http_share::websocket::{
        ClientMessage, ServerMessage, UploadState, respond_to_upload,
    };
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

        let router =
            create_router_with_websocket(token, tx, upload_state.clone(), download_dir.clone());

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
        let metadata = tokio::fs::metadata(&target_file)
            .await
            .expect("File not found");

        println!("File size on disk: {}", metadata.len());
        assert!(
            metadata.len() <= claimed_size,
            "File on disk ({}) is larger than claimed size ({})",
            metadata.len(),
            claimed_size
        );

        // Cleanup
        let _ = tokio::fs::remove_file(&target_file).await;
        let _ = tokio::fs::remove_dir(&download_dir).await;
    }

    #[tokio::test]
    async fn test_max_active_uploads_limit() {
        use p2p_core::http_share::websocket::MAX_ACTIVE_UPLOADS;

        // Setup
        let token = "test_token_active_limit";
        let (tx, mut rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::new());
        let download_dir = std::env::temp_dir().join("p2p_test_active_limit");
        let _ = tokio::fs::create_dir_all(&download_dir).await;

        let router =
            create_router_with_websocket(token, tx, upload_state.clone(), download_dir.clone());
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

        let mut clients = Vec::new();
        // Try to start MAX + 1 uploads
        let count = MAX_ACTIVE_UPLOADS + 1;

        for i in 0..count {
            let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
            let (mut write, read) = ws_stream.split();

            // Send FileInfo
            let msg = ClientMessage::FileInfo {
                file_name: format!("file_{}.txt", i),
                file_size: 1024,
            };
            write
                .send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
                .await
                .unwrap();

            // Wait for UploadRequest event
            let request_id = tokio::time::timeout(tokio::time::Duration::from_secs(2), async {
                // We must process ALL events from the channel until we find ours or timeout
                // Note: Since we are in a loop, old events might be in the channel?
                // The rx is unique to this test setup (created fresh).
                // But events from previous iterations (if any?) would be consumed.
                // Since we consume the event for client i in iteration i, it should be fine.
                while let Some(evt) = rx.recv().await {
                    if let AppEvent::UploadRequest { request_id, .. } = evt {
                        return Some(request_id);
                    }
                }
                None
            })
            .await
            .expect("Timeout waiting for UploadRequest")
            .expect("No UploadRequest");

            // Accept
            respond_to_upload(&upload_state, &request_id, true).await;

            // Store client to keep connection alive and read response later
            clients.push((write, read, i));
        }

        // Verify results
        let mut accepted_count = 0;
        let mut rejected_count = 0;

        for (_write, mut read, _i) in clients {
            // Read response
            // We expect immediate response after acceptance
            if let Ok(Some(Ok(Message::Text(text)))) =
                tokio::time::timeout(tokio::time::Duration::from_secs(1), read.next()).await
            {
                let server_msg: ServerMessage = serde_json::from_str(&text).unwrap();
                match server_msg {
                    ServerMessage::Accepted { .. } => accepted_count += 1,
                    ServerMessage::Rejected { reason } => {
                        rejected_count += 1;
                        println!("Rejected reason: {}", reason);
                    }
                    _ => println!("Unexpected message: {:?}", server_msg),
                }
            }
        }

        assert_eq!(
            accepted_count, MAX_ACTIVE_UPLOADS,
            "Should accept exactly MAX_ACTIVE_UPLOADS ({})",
            MAX_ACTIVE_UPLOADS
        );
        assert_eq!(rejected_count, 1, "Should reject exactly 1");

        // Cleanup
        let _ = tokio::fs::remove_dir_all(&download_dir).await;
    }
}
