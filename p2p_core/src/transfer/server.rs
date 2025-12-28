use crate::{AppEvent, pairing};
use anyhow::{Result, anyhow};
use quinn::Endpoint;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::sync::mpsc;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::protocol::{TransferMsg, recv_msg, send_msg};

// Limit concurrent pairing attempts to prevent UI spam and brute force
static ACTIVE_PAIRING_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
const MAX_CONCURRENT_PAIRINGS: usize = 3;

struct PairingGuard;

impl PairingGuard {
    fn try_new() -> Option<Self> {
        let count = ACTIVE_PAIRING_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
        if count >= MAX_CONCURRENT_PAIRINGS {
            ACTIVE_PAIRING_ATTEMPTS.fetch_sub(1, Ordering::SeqCst);
            None
        } else {
            Some(Self)
        }
    }
}

impl Drop for PairingGuard {
    fn drop(&mut self) {
        ACTIVE_PAIRING_ATTEMPTS.fetch_sub(1, Ordering::SeqCst);
    }
}
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

                    while let Ok((mut send_stream, mut recv_stream)) = connection.accept_bi().await
                    {
                        let event_tx = event_tx.clone();
                        let download_dir = download_dir.clone();

                        tokio::spawn(async move {
                            // Read first message to determine type
                            match recv_msg(&mut recv_stream).await {
                                Ok(msg) => {
                                    match msg {
                                        TransferMsg::PairingRequest { peer_id, peer_name } => {
                                            // Handle Handshake
                                            if let Err(e) = handle_verification_handshake(
                                                &mut send_stream,
                                                &mut recv_stream,
                                                &event_tx,
                                                remote_addr,
                                                peer_id,
                                                peer_name,
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
    peer_id: String,
    peer_name: String,
) -> Result<()> {
    if pairing::is_paired(&peer_id) {
        send_msg(send, &TransferMsg::PairingAccepted).await?;
        let _ = event_tx
            .send(AppEvent::PairingResult {
                success: true,
                peer_name: peer_name.clone(),
                message: "Previously paired".to_string(),
            })
            .await;
        return Ok(());
    }

    // Limit concurrent pairing attempts
    let _guard = match PairingGuard::try_new() {
        Some(g) => g,
        None => {
            send_msg(
                send,
                &TransferMsg::VerificationFailed {
                    message: "Too many active pairing attempts".to_string(),
                },
            )
            .await?;
            return Err(anyhow!("Too many active pairing attempts"));
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

    let msg = recv_msg(recv).await?;
    match msg {
        TransferMsg::VerificationCode {
            code: received_code,
        } => {
            // Artificial delay to prevent brute-force attacks and timing analysis
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            if received_code == code {
                pairing::add_pairing(&peer_id, &peer_name);
                send_msg(send, &TransferMsg::VerificationSuccess).await?;
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
