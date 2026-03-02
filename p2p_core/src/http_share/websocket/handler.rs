//! WebSocket connection handler

use super::messages::{
    MAX_CONNECTIONS, MAX_CONNECTIONS_PER_IP, ServerMessage, USER_RESPONSE_TIMEOUT_SECS,
};
use super::state::{ActiveUploadGuard, WebSocketState};
use super::utils::{cleanup_pending, create_secure_file, validate_file_info, wait_for_file_info};
use crate::AppEvent;
use crate::transfer::utils::sanitize_file_name;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::{io::AsyncWriteExt, sync::oneshot};
use uuid::Uuid;

/// RAII guard to decrement connection count on drop
struct ConnectionGuard {
    state: Arc<WebSocketState>,
    client_ip: String,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.state.connection_count.fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut counts) = self.state.ip_counts.lock() {
            if let Some(count) = counts.get_mut(&self.client_ip) {
                if *count > 0 {
                    *count -= 1;
                }
                if *count == 0 {
                    counts.remove(&self.client_ip);
                }
            }
        }
    }
}

/// Ping interval for keeping WebSocket connection alive (5 seconds)
/// Mobile browsers may have stricter timeouts, so we ping more frequently
const PING_INTERVAL_SECS: u64 = 5;

/// Timeout for the initial handshake to prevent DoS (10 seconds)
const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

/// Handle WebSocket connection
pub async fn handle_socket(socket: WebSocket, state: Arc<WebSocketState>, client_ip: String) {
    let (mut sender, mut receiver) = socket.split();

    // Check IP limit
    {
        // We use a block here to drop the lock immediately after checking/incrementing
        let count = {
            let mut counts = state.ip_counts.lock().unwrap_or_else(|e| e.into_inner());
            let count = counts.entry(client_ip.clone()).or_insert(0);
            if *count >= MAX_CONNECTIONS_PER_IP {
                *count // Return the value to check outside
            } else {
                *count += 1;
                0 // Indicates success
            }
        };

        if count >= MAX_CONNECTIONS_PER_IP {
            tracing::warn!(
                "Rejecting connection from {}: Too many connections from this IP ({})",
                client_ip,
                count + 1
            );
            // We can't easily send a message and close gracefully here if we want to be fast,
            // but let's try to be nice.
            // Note: If we just return, the socket is dropped and closed.
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: "Too many connections from this IP".to_string(),
                    })
                    .unwrap_or_else(|_| {
                        "{\"type\":\"error\",\"message\":\"Internal serialization error\"}"
                            .to_string()
                    })
                    .into(),
                ))
                .await;
            return;
        }
    }

    // Check connection limit
    let current_connections = state.connection_count.fetch_add(1, Ordering::SeqCst);
    if current_connections >= MAX_CONNECTIONS {
        tracing::warn!(
            "Rejecting connection from {}: Too many concurrent connections ({})",
            client_ip,
            current_connections + 1
        );
        // Decrement IP count since we are rejecting
        if let Ok(mut counts) = state.ip_counts.lock() {
            if let Some(count) = counts.get_mut(&client_ip) {
                if *count > 0 {
                    *count -= 1;
                }
                // Optional: cleanup if 0
                if *count == 0 {
                    counts.remove(&client_ip);
                }
            }
        }

        state.connection_count.fetch_sub(1, Ordering::SeqCst);
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Error {
                    message: "Too many concurrent connections".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string()
                })
                .into(),
            ))
            .await;
        return;
    }

    let _connection_guard = ConnectionGuard {
        state: state.clone(),
        client_ip: client_ip.clone(),
    };

    tracing::info!("WebSocket connection established from: {}", client_ip);

    // Wait for file info message with timeout
    let file_info = match tokio::time::timeout(
        tokio::time::Duration::from_secs(HANDSHAKE_TIMEOUT_SECS),
        wait_for_file_info(&mut receiver),
    )
    .await
    {
        Ok(Some(info)) => info,
        Ok(None) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: "Expected file_info message".to_string(),
                    })
                    .unwrap_or_else(|_| {
                        "{\"type\":\"error\",\"message\":\"Internal serialization error\"}"
                            .to_string()
                    })
                    .into(),
                ))
                .await;
            return;
        }
        Err(_) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: "Handshake timed out".to_string(),
                    })
                    .unwrap_or_else(|_| {
                        "{\"type\":\"error\",\"message\":\"Internal serialization error\"}"
                            .to_string()
                    })
                    .into(),
                ))
                .await;
            return;
        }
    };

    let (raw_file_name, file_size) = file_info;

    // Validate file info
    if let Err(e) = validate_file_info(&raw_file_name, file_size) {
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Error { message: e })
                    .unwrap_or_else(|_| {
                        "{\"type\":\"error\",\"message\":\"Internal serialization error\"}"
                            .to_string()
                    })
                    .into(),
            ))
            .await;
        return;
    }

    // Sanitize filename to prevent directory traversal
    let file_name = sanitize_file_name(&raw_file_name);

    // Use full UUID entropy (128 bits) instead of 8 chars (32 bits)
    // to prevent brute-force attacks on request tokens.
    let request_id = Uuid::new_v4().simple().to_string();

    // Create response channel
    let (response_tx, response_rx) = oneshot::channel();
    // Pin response_rx so we can poll it in a loop
    tokio::pin!(response_rx);

    // Store pending upload
    if !state
        .upload_state
        .try_add_request(request_id.clone(), response_tx)
        .await
    {
        tracing::warn!(
            "Rejecting upload from {}: Too many pending uploads",
            client_ip
        );
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Error {
                    message: "Too many pending uploads".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string()
                })
                .into(),
            ))
            .await;
        return;
    }

    // Send upload request event to GUI
    let _ = state
        .event_tx
        .send(AppEvent::UploadRequest {
            request_id: request_id.clone(),
            file_name: file_name.clone(),
            file_size,
            from_ip: client_ip.clone(),
        })
        .await;

    // Wait for user response with timeout or client disconnect
    let accepted = loop {
        tokio::select! {
            // 1. User response from GUI
            res = &mut response_rx => {
                match res {
                    Ok(val) => break val,
                    Err(_) => {
                        // Channel closed (internal error)
                        let _ = sender
                            .send(Message::Text(
                                serde_json::to_string(&ServerMessage::Error {
                                    message: "Internal error".to_string(),
                                })
                                .unwrap_or_else(|_| "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string())
                                .into(),
                            ))
                            .await;
                        cleanup_pending(&state.upload_state, &request_id).await;
                        // Notify GUI to close popup
                        let _ = state.event_tx.send(AppEvent::UploadRequestCancelled { request_id: request_id.clone() }).await;
                        return;
                    }
                }
            }
            // 2. Client disconnected (Socket closed) or sent unexpected message
            msg = receiver.next() => {
                 match msg {
                    Some(Ok(Message::Close(_))) | None => {
                        // Client disconnected
                         cleanup_pending(&state.upload_state, &request_id).await;
                         let _ = state.event_tx.send(AppEvent::UploadRequestCancelled { request_id: request_id.clone() }).await;
                         return;
                    }
                    Some(Err(e)) => {
                        tracing::error!("WebSocket error: {}", e);
                        cleanup_pending(&state.upload_state, &request_id).await;
                        let _ = state.event_tx.send(AppEvent::UploadRequestCancelled { request_id: request_id.clone() }).await;
                        return;
                    }
                    _ => {
                        // Ignore other messages (e.g. Ping/Pong) or unexpected data
                        continue;
                    }
                 }
            }
            // 3. Timeout
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(USER_RESPONSE_TIMEOUT_SECS)) => {
                let _ = sender
                    .send(Message::Text(
                        serde_json::to_string(&ServerMessage::Rejected {
                            reason: "Request timed out".to_string(),
                        })
                        .unwrap_or_else(|_| "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string())
                        .into(),
                    ))
                    .await;
                cleanup_pending(&state.upload_state, &request_id).await;
                // Notify GUI to close popup
                let _ = state.event_tx.send(AppEvent::UploadRequestCancelled { request_id: request_id.clone() }).await;
                return;
            }
        }
    };

    // Clean up pending
    cleanup_pending(&state.upload_state, &request_id).await;

    if !accepted {
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Rejected {
                    reason: "User rejected the upload".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string()
                })
                .into(),
            ))
            .await;
        return;
    }

    // Check active limit using atomic acquire
    if !state.upload_state.try_acquire_active_slot() {
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Rejected {
                    reason: "Too many active uploads".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string()
                })
                .into(),
            ))
            .await;
        return;
    }

    let _active_guard = ActiveUploadGuard {
        state: state.upload_state.clone(),
    };

    // Send accepted message
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&ServerMessage::Accepted {
                request_id: request_id.clone(),
            })
            .unwrap_or_else(|_| {
                "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string()
            })
            .into(),
        ))
        .await;

    // Prepare download path
    let download_dir = state.download_dir.clone();
    if let Err(e) = tokio::fs::create_dir_all(&download_dir).await {
        tracing::error!("Failed to create download dir: {}", e);
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Error {
                    message: "Cannot create download directory".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string()
                })
                .into(),
            ))
            .await;
        return;
    }

    let file_path = download_dir.join(&file_name);
    let mut file = match create_secure_file(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            tracing::error!("Failed to create secure file: {}", e);
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: "Cannot create file".to_string(),
                    })
                    .unwrap_or_else(|_| {
                        "{\"type\":\"error\",\"message\":\"Internal serialization error\"}"
                            .to_string()
                    })
                    .into(),
                ))
                .await;
            return;
        }
    };

    // Receive binary chunks with periodic ping to keep connection alive
    let mut received_bytes: u64 = 0;
    let mut last_progress_update = std::time::Instant::now();

    // Create ping interval (especially important for mobile browsers)
    let mut ping_interval =
        tokio::time::interval(tokio::time::Duration::from_secs(PING_INTERVAL_SECS));
    ping_interval.tick().await; // Skip first immediate tick

    loop {
        tokio::select! {
            // Send periodic ping to keep connection alive
            _ = ping_interval.tick() => {
                tracing::info!("Sending WebSocket ping to keep connection alive");
                if let Err(e) = sender.send(Message::Ping(bytes::Bytes::new())).await {
                    tracing::error!("Failed to send ping: {}", e);
                    break;
                }
            }
            // Receive messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Binary(data))) => {
                        let remaining = file_size.saturating_sub(received_bytes);

                        // Check if we already have all data
                        if remaining == 0 {
                            let _ = sender
                                .send(Message::Text(
                                    serde_json::to_string(&ServerMessage::Error {
                                        message: "Received more data than declared".to_string(),
                                    })
                                    .unwrap_or_else(|_| "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string())
                                    .into(),
                                ))
                                .await;
                            return; // Stop processing
                        }

                        let data_len = data.len() as u64;
                        let (to_write, overflow) = if data_len > remaining {
                            (&data[..remaining as usize], true)
                        } else {
                            (&data[..], false)
                        };

                        if let Err(e) = file.write_all(to_write).await {
                            tracing::error!("Failed to write to file: {}", e);
                            let _ = sender
                                .send(Message::Text(
                                    serde_json::to_string(&ServerMessage::Error {
                                        message: "Write error occurred".to_string(),
                                    })
                                    .unwrap_or_else(|_| "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string())
                                    .into(),
                                ))
                                .await;
                            break;
                        }

                        received_bytes += to_write.len() as u64;

                        // Send progress every 100ms or at completion
                        if last_progress_update.elapsed().as_millis() > 100 || received_bytes >= file_size
                        {
                            let _ = sender
                                .send(Message::Text(
                                    serde_json::to_string(&ServerMessage::Progress {
                                        received_bytes,
                                    })
                                    .unwrap_or_else(|_| "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string())
                                    .into(),
                                ))
                                .await;

                            // Also send to GUI
                            let _ = state
                                .event_tx
                                .send(AppEvent::UploadProgress {
                                    request_id: request_id.clone(),
                                    received_bytes,
                                    total_bytes: file_size,
                                })
                                .await;

                            last_progress_update = std::time::Instant::now();
                        }

                        if overflow {
                            tracing::warn!(
                                "Client {} sent more data than declared. Truncated.",
                                client_ip
                            );
                            // We stop accepting more data as we have the full file
                            break;
                        }

                        if received_bytes >= file_size {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) => {
                        tracing::info!("Client closed WebSocket connection");
                        break;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        tracing::info!("Received pong response from client");
                        // Connection is alive, continue
                    }
                    Some(Err(e)) => {
                        tracing::error!("WebSocket error during upload: {}", e);
                        break;
                    }
                    None => {
                        tracing::warn!("WebSocket stream ended unexpectedly");
                        break;
                    }
                    _ => {
                        // Ignore other message types (e.g., Text, Ping)
                    }
                }
            }
        }
    }

    // Finalize
    if let Err(e) = file.flush().await {
        tracing::error!("Failed to flush file: {}", e);
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Error {
                    message: "Flush error occurred".to_string(),
                })
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string()
                })
                .into(),
            ))
            .await;
        return;
    }

    let saved_path = file_path.to_string_lossy().to_string();

    // Send complete message
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&ServerMessage::Complete)
                .unwrap_or_else(|_| {
                    "{\"type\":\"error\",\"message\":\"Internal serialization error\"}".to_string()
                })
                .into(),
        ))
        .await;

    // Notify GUI
    let _ = state
        .event_tx
        .send(AppEvent::UploadCompleted {
            file_name,
            saved_path,
        })
        .await;

    tracing::info!(
        "Upload complete: {} bytes from {}",
        received_bytes,
        client_ip
    );

    // Close WebSocket gracefully to avoid abnormal closure (code 1006)
    let _ = sender.send(Message::Close(None)).await;

    // Give client time to process close frame
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
}
