use crate::{AppEvent, pairing};
use anyhow::{Result, anyhow};
use quinn::Endpoint;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::sync::mpsc;

use super::protocol::{TransferMsg, recv_msg, send_msg};
use super::receiver::receive_file;

/// Run the QUIC server to accept incoming file transfers
pub async fn run_server(
    endpoint: Endpoint,
    event_tx: mpsc::Sender<AppEvent>,
    download_dir: PathBuf,
) {
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] QUIC Server running, waiting for connections...".to_string(),
        ))
        .await;
    while let Some(incoming) = endpoint.accept().await {
        let _ = event_tx
            .send(AppEvent::Status(
                "[DEBUG] Incoming connection received.".to_string(),
            ))
            .await;
        let event_tx = event_tx.clone();
        let download_dir = download_dir.clone();

        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    let remote_addr = connection.remote_address();
                    // Accept the bidirectional stream for handshake
                    match connection.accept_bi().await {
                        Ok((mut send_stream, mut recv_stream)) => {
                            if let Err(e) = handle_verification_handshake(
                                &mut send_stream,
                                &mut recv_stream,
                                &event_tx,
                                remote_addr,
                            )
                            .await
                            {
                                let _ = event_tx
                                    .send(AppEvent::Error(format!(
                                        "Verification error ({}): {}",
                                        remote_addr, e
                                    )))
                                    .await;
                                return; // Stop if handshake fails
                            }
                        }
                        Err(e) => {
                            let _ = event_tx
                                .send(AppEvent::Error(format!(
                                    "Cannot open verification channel ({}): {}",
                                    remote_addr, e
                                )))
                                .await;
                            return;
                        }
                    }

                    // Handshake success, now accept file streams
                    let _ = event_tx
                        .send(AppEvent::Status(format!(
                            "Connected and verified: {}",
                            remote_addr
                        )))
                        .await;

                    // Accept uni-directional stream for file data
                    let _ = event_tx
                        .send(AppEvent::Status(
                            "[DEBUG] Waiting for file streams...".to_string(),
                        ))
                        .await;
                    while let Ok(mut recv_stream) = connection.accept_uni().await {
                        let _ = event_tx
                            .send(AppEvent::Status(
                                "[DEBUG] New file stream accepted.".to_string(),
                            ))
                            .await;
                        let event_tx = event_tx.clone();
                        let download_dir = download_dir.clone();

                        tokio::spawn(async move {
                            if let Err(e) =
                                receive_file(&mut recv_stream, &download_dir, &event_tx).await
                            {
                                let _ = event_tx
                                    .send(AppEvent::Error(format!("Receive file error: {}", e)))
                                    .await;
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

/// Handle the verification handshake on the receiver side
async fn handle_verification_handshake(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    event_tx: &mpsc::Sender<AppEvent>,
    remote_addr: SocketAddr,
) -> Result<()> {
    // 1. Wait for PairingRequest
    let msg = recv_msg(recv).await?;
    let (peer_id, peer_name) = match msg {
        TransferMsg::PairingRequest { peer_id, peer_name } => (peer_id, peer_name),
        _ => return Err(anyhow!("Expected PairingRequest, got {:?}", msg)),
    };

    // 2. Check if already paired
    if pairing::is_paired(&peer_id) {
        // Already paired -> Accept
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

    // 3. Not paired -> Require Verification
    // Generate code
    let code = pairing::generate_verification_code();

    // Notify UI to show code
    let _ = event_tx
        .send(AppEvent::ShowVerificationCode {
            code: code.clone(),
            from_ip: remote_addr.ip().to_string(),
            from_name: peer_name.clone(),
        })
        .await;

    // Send challenge to sender
    send_msg(send, &TransferMsg::VerificationRequired).await?;

    // 4. Wait for VerificationCode
    let msg = recv_msg(recv).await?;
    match msg {
        TransferMsg::VerificationCode {
            code: received_code,
        } => {
            if received_code == code {
                // Success
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
                // Failed
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
