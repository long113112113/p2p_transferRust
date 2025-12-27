//! WebSocket connection handler

use super::messages::{ServerMessage, USER_RESPONSE_TIMEOUT_SECS};
use super::state::{PendingUpload, WebSocketState};
use super::utils::{cleanup_pending, wait_for_file_info};
use crate::AppEvent;
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::{fs::File, io::AsyncWriteExt, sync::oneshot};
use uuid::Uuid;

/// Handle WebSocket connection
pub async fn handle_socket(socket: WebSocket, state: Arc<WebSocketState>, client_ip: String) {
    let (mut sender, mut receiver) = socket.split();

    // Wait for file info message
    let file_info = match wait_for_file_info(&mut receiver).await {
        Some(info) => info,
        None => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: "Expected file_info message".to_string(),
                    })
                    .unwrap()
                    .into(),
                ))
                .await;
            return;
        }
    };

    let (raw_file_name, file_size) = file_info;

    // Sanitize filename to prevent directory traversal
    let file_name = std::path::Path::new(&raw_file_name)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown_file.bin".to_string());
    let request_id = Uuid::new_v4().to_string()[..8].to_string();

    // Create response channel
    let (response_tx, response_rx) = oneshot::channel();
    // Pin response_rx so we can poll it in a loop
    tokio::pin!(response_rx);

    // Store pending upload
    {
        let mut pending = state.upload_state.pending.write().await;
        pending.insert(request_id.clone(), PendingUpload { response_tx });
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
                                .unwrap()
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
                        .unwrap()
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
                .unwrap()
                .into(),
            ))
            .await;
        return;
    }

    // Send accepted message
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&ServerMessage::Accepted {
                request_id: request_id.clone(),
            })
            .unwrap()
            .into(),
        ))
        .await;

    // Prepare download path
    let download_dir = state.download_dir.clone();
    if let Err(e) = tokio::fs::create_dir_all(&download_dir).await {
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Error {
                    message: format!("Cannot create download dir: {}", e),
                })
                .unwrap()
                .into(),
            ))
            .await;
        return;
    }

    let file_path = download_dir.join(&file_name);
    let mut file = match File::create(&file_path).await {
        Ok(f) => f,
        Err(e) => {
            let _ = sender
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Error {
                        message: format!("Cannot create file: {}", e),
                    })
                    .unwrap()
                    .into(),
                ))
                .await;
            return;
        }
    };

    // Receive binary chunks
    let mut received_bytes: u64 = 0;
    let mut last_progress_update = std::time::Instant::now();

    while let Some(msg) = receiver.next().await {
        match msg {
            Ok(Message::Binary(data)) => {
                if let Err(e) = file.write_all(&data).await {
                    let _ = sender
                        .send(Message::Text(
                            serde_json::to_string(&ServerMessage::Error {
                                message: format!("Write error: {}", e),
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await;
                    return;
                }

                received_bytes += data.len() as u64;

                // Send progress every 100ms or at completion
                if last_progress_update.elapsed().as_millis() > 100 || received_bytes >= file_size {
                    let _ = sender
                        .send(Message::Text(
                            serde_json::to_string(&ServerMessage::Progress { received_bytes })
                                .unwrap()
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

                if received_bytes >= file_size {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Err(_) => break,
            _ => {}
        }
    }

    // Finalize
    if let Err(e) = file.flush().await {
        let _ = sender
            .send(Message::Text(
                serde_json::to_string(&ServerMessage::Error {
                    message: format!("Flush error: {}", e),
                })
                .unwrap()
                .into(),
            ))
            .await;
        return;
    }

    let saved_path = file_path.to_string_lossy().to_string();

    // Send complete message
    let _ = sender
        .send(Message::Text(
            serde_json::to_string(&ServerMessage::Complete {
                saved_path: saved_path.clone(),
            })
            .unwrap()
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
}
