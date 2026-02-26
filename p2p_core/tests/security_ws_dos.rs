use axum::serve;
use futures_util::StreamExt;
use p2p_core::http_share::server::create_router_with_websocket;
use p2p_core::http_share::websocket::{UploadState, MAX_CONNECTIONS};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn test_max_concurrent_connections_dos() {
    // Setup
    let token = "test_token_dos";
    let (tx, _rx) = mpsc::channel(100);
    let upload_state = Arc::new(UploadState::default());
    let download_dir = PathBuf::from(".");
    let router = create_router_with_websocket(token, tx, upload_state, download_dir);

    // Bind to random port
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // Spawn server
    tokio::spawn(async move {
        serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // Give server a moment to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let ws_url = format!("ws://127.0.0.1:{}/{}/ws", port, token);

    // Try to connect MAX + 5 clients
    let limit = MAX_CONNECTIONS + 5;

    println!("Testing with MAX_CONNECTIONS = {}", MAX_CONNECTIONS);

    let mut handles = Vec::new();
    for i in 0..limit {
        let ws_url = ws_url.clone();
        handles.push(tokio::spawn(async move {
            match connect_async(&ws_url).await {
                Ok((ws_stream, _)) => {
                    let (write, mut read) = ws_stream.split();

                    // Read first message (should be rejection error if limit reached)
                    // We use timeout to verify immediate rejection or acceptance
                    let msg_result = tokio::time::timeout(tokio::time::Duration::from_millis(2000), read.next()).await;
                    (i, Ok((write, read, msg_result)))
                },
                Err(e) => (i, Err(e))
            }
        }));
    }

    let results = futures_util::future::join_all(handles).await;

    let mut rejected_count = 0;
    let mut accepted_count = 0;
    let mut _clients = Vec::new(); // Keep connections alive

    for res in results {
        let (i, result) = res.unwrap();
        match result {
            Ok((write, read, msg_result)) => {
                match msg_result {
                    Ok(Some(Ok(Message::Text(text)))) => {
                        if text.contains("Too many concurrent connections") {
                            println!("Client {} rejected: {}", i, text);
                            rejected_count += 1;
                        } else {
                            // Accepted (got other message?)
                             _clients.push((write, read));
                             accepted_count += 1;
                        }
                    },
                    Ok(Some(Ok(Message::Close(_)))) => {
                        println!("Client {} closed immediately", i);
                        rejected_count += 1;
                    },
                    Err(_) => {
                        // Timeout - connection accepted and idle
                        _clients.push((write, read));
                        accepted_count += 1;
                    },
                    _ => {
                        // Other cases (error, none, etc)
                         println!("Client {} other result: {:?}", i, msg_result);
                         rejected_count += 1;
                    }
                }
            },
            Err(e) => {
                println!("Client {} failed to connect: {}", i, e);
                rejected_count += 1;
            }
        }
    }

    println!("Accepted: {}, Rejected: {}", accepted_count, rejected_count);

    assert!(rejected_count >= 1, "Should have rejected at least some connections. Rejected: {}", rejected_count);
    assert!(accepted_count <= MAX_CONNECTIONS, "Should not accept more than MAX_CONNECTIONS. Accepted: {}", accepted_count);
}
