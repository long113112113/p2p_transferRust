use crate::{AppEvent, pairing};
use anyhow::{Result, anyhow};
use quinn::Endpoint;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

use super::protocol::{TransferMsg, recv_msg, send_msg};
use super::receiver::receive_file;

/// Run the QUIC server to accept incoming file transfers
pub async fn run_server(
    endpoint: Endpoint,
    event_tx: mpsc::Sender<AppEvent>,
    download_dir: PathBuf,
) {
    while let Some(incoming) = endpoint.accept().await {
        let event_tx = event_tx.clone();
        let download_dir = download_dir.clone();

        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    let remote_addr = connection.remote_address();
                    let is_authenticated = Arc::new(AtomicBool::new(false));

                    while let Ok((mut send_stream, mut recv_stream)) = connection.accept_bi().await
                    {
                        let event_tx = event_tx.clone();
                        let download_dir = download_dir.clone();
                        let is_authenticated = is_authenticated.clone();

                        tokio::spawn(async move {
                            // Read first message to determine type
                            match recv_msg(&mut recv_stream).await {
                                Ok(msg) => {
                                    match msg {
                                        TransferMsg::PairingRequest {
                                            endpoint_id,
                                            peer_name,
                                        } => {
                                            // Handle Handshake
                                            if let Err(e) = handle_verification_handshake(
                                                &mut send_stream,
                                                &mut recv_stream,
                                                &event_tx,
                                                remote_addr,
                                                endpoint_id,
                                                peer_name,
                                                &is_authenticated,
                                            )
                                            .await
                                            {
                                                let _ = event_tx
                                                    .send(AppEvent::Error(format!(
                                                        "Verification error ({}): {}",
                                                        remote_addr, e
                                                    )))
                                                    .await;
                                            }
                                        }
                                        TransferMsg::FileMetadata { info } => {
                                            // Check authentication
                                            if !is_authenticated.load(Ordering::SeqCst) {
                                                tracing::warn!(
                                                    "Rejected unauthenticated upload from {}",
                                                    remote_addr
                                                );
                                                let _ = send_msg(
                                                    &mut send_stream,
                                                    &TransferMsg::VerificationFailed {
                                                        message: "Unauthenticated transfer rejected"
                                                            .to_string(),
                                                    },
                                                )
                                                .await;
                                                return;
                                            }

                                            // Handle File Transfer

                                            if let Err(e) = receive_file(
                                                &mut send_stream,
                                                &mut recv_stream,
                                                &download_dir,
                                                &event_tx,
                                                info,
                                            )
                                            .await
                                            {
                                                let _ = event_tx
                                                    .send(AppEvent::Error(format!(
                                                        "Receive file error: {}",
                                                        e
                                                    )))
                                                    .await;
                                            }
                                        }
                                        _ => {
                                            let _ = event_tx
                                                .send(AppEvent::Error(format!(
                                                    "Unexpected first message from {}: {:?}",
                                                    remote_addr, msg
                                                )))
                                                .await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = event_tx
                                        .send(AppEvent::Error(format!(
                                            "Error reading first message from {}: {}",
                                            remote_addr, e
                                        )))
                                        .await;
                                }
                            }
                        });
                    }
                }
                Err(e) => {
                    let _ = event_tx
                        .send(AppEvent::Error(format!("QUIC connection error: {}", e)))
                        .await;
                }
            }
        });
    }
}

async fn handle_verification_handshake(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    event_tx: &mpsc::Sender<AppEvent>,
    remote_addr: SocketAddr,
    endpoint_id: String,
    peer_name: String,
    is_authenticated: &Arc<AtomicBool>,
) -> Result<()> {
    if pairing::is_paired(&endpoint_id) {
        send_msg(send, &TransferMsg::PairingAccepted).await?;
        is_authenticated.store(true, Ordering::SeqCst);
        let _ = event_tx
            .send(AppEvent::PairingResult {
                success: true,
                peer_name: peer_name.clone(),
                message: "Previously paired".to_string(),
            })
            .await;
        return Ok(());
    }

    // Enforce concurrency limit to prevent brute-force attacks
    // The guard is held until the end of the function scope
    let _guard = match pairing::PairingGuard::try_acquire() {
        Some(g) => g,
        None => {
            send_msg(
                send,
                &TransferMsg::VerificationFailed {
                    message: "Too many pending verification attempts".to_string(),
                },
            )
            .await?;
            // We don't notify the user to avoid spam, but we log it
            tracing::warn!(
                "Rejected pairing from {}: Too many pending attempts",
                remote_addr
            );
            return Err(anyhow!("Too many pending attempts"));
        }
    };

    let code = pairing::generate_verification_code();

    let _ = event_tx
        .send(AppEvent::ShowVerificationCode {
            code: code.clone(),
            from_ip: remote_addr.ip().to_string(),
            from_name: peer_name.clone(),
        })
        .await;

    send_msg(send, &TransferMsg::VerificationRequired).await?;

    let msg = match tokio::time::timeout(
        super::constants::get_pairing_timeout(),
        recv_msg(recv),
    )
    .await
    {
        Ok(res) => res?,
        Err(_) => {
            // Timeout
            let _ = send_msg(
                send,
                &TransferMsg::VerificationFailed {
                    message: "Verification timed out".to_string(),
                },
            )
            .await;
            return Err(anyhow!("Verification timed out"));
        }
    };

    match msg {
        TransferMsg::VerificationCode {
            code: received_code,
        } => {
            // Add delay to slow down brute-force attacks
            // This holds the connection (and the guard) for 2 seconds
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;

            if received_code == code {
                pairing::add_pairing(&endpoint_id, &peer_name);
                send_msg(send, &TransferMsg::VerificationSuccess).await?;
                is_authenticated.store(true, Ordering::SeqCst);
                let _ = event_tx
                    .send(AppEvent::PairingResult {
                        success: true,
                        peer_name,
                        message: "Verification successful".to_string(),
                    })
                    .await;
                Ok(())
            } else {
                send_msg(
                    send,
                    &TransferMsg::VerificationFailed {
                        message: "Invalid code".to_string(),
                    },
                )
                .await?;
                let _ = event_tx
                    .send(AppEvent::PairingResult {
                        success: false,
                        peer_name,
                        message: "Invalid verification code".to_string(),
                    })
                    .await;
                Err(anyhow!("Verification failed: Wrong code"))
            }
        }
        _ => Err(anyhow!("Expected VerificationCode, got {:?}", msg)),
    }
}
