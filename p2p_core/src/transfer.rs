//! QUIC-based file transfer module using quinn.
//!
//! This module provides:
//! - Self-signed certificate generation
//! - QUIC server endpoint (to receive files)
//! - QUIC client endpoint (to send files)
//! - Verification handshake with 4-digit code

use crate::pairing;
use crate::{AppEvent, FileInfo};
use anyhow::{Result, anyhow};
use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// Default QUIC port for file transfer
pub const TRANSFER_PORT: u16 = 9000;

/// Buffer size for file transfer (64KB)
const BUFFER_SIZE: usize = 64 * 1024;

/// Protocol messages for transfer handshake
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransferMsg {
    /// Sender initiates connection with their identity
    PairingRequest { peer_id: String, peer_name: String },
    /// Receiver responds: already paired, proceed
    PairingAccepted,
    /// Receiver responds: need verification, show code to user
    VerificationRequired,
    /// Sender submits the 4-digit code
    VerificationCode { code: String },
    /// Verification successful, can proceed with transfer
    VerificationSuccess,
    /// Verification failed
    VerificationFailed { message: String },
    /// File transfer metadata
    FileMetadata { info: FileInfo },
    /// Ready to receive file data
    ReadyForData,
}

/// Generate a self-signed certificate for QUIC
pub fn generate_self_signed_cert()
-> Result<(Vec<CertificateDer<'static>>, PrivatePkcs8KeyDer<'static>)> {
    let certified_key = generate_simple_self_signed(vec!["localhost".to_string()])?;
    let key = PrivatePkcs8KeyDer::from(certified_key.signing_key.serialize_der());
    let cert_der = CertificateDer::from(certified_key.cert.der().to_vec());
    Ok((vec![cert_der], key))
}

/// Send a protocol message over a bidirectional stream
async fn send_msg(send: &mut quinn::SendStream, msg: &TransferMsg) -> Result<()> {
    let json = serde_json::to_vec(msg)?;
    let len = (json.len() as u32).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(&json).await?;
    Ok(())
}

/// Receive a protocol message from a bidirectional stream
async fn recv_msg(recv: &mut quinn::RecvStream) -> Result<TransferMsg> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;

    let msg: TransferMsg = serde_json::from_slice(&buf)?;
    Ok(msg)
}

/// Create a QUIC server endpoint
pub fn make_server_endpoint(bind_addr: SocketAddr) -> Result<Endpoint> {
    let (certs, key) = generate_self_signed_cert()?;

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key.into())?;

    server_crypto.alpn_protocols = vec![b"p2p-transfer".to_vec()];

    let mut server_config = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto)?,
    ));

    // Configure transport
    let mut transport_config = TransportConfig::default();
    transport_config.max_idle_timeout(Some(Duration::from_secs(30).try_into()?));
    server_config.transport_config(Arc::new(transport_config));

    let endpoint = Endpoint::server(server_config, bind_addr)?;
    Ok(endpoint)
}

/// Create a QUIC client endpoint (skip certificate verification for P2P)
pub fn make_client_endpoint() -> Result<Endpoint> {
    // Create client config that skips certificate verification (for self-signed certs)
    let mut crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    crypto.alpn_protocols = vec![b"p2p-transfer".to_vec()];

    let mut client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(crypto)?,
    ));

    // Configure transport
    let mut transport_config = TransportConfig::default();
    transport_config.max_idle_timeout(Some(Duration::from_secs(30).try_into()?));
    client_config.transport_config(Arc::new(transport_config));

    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    Ok(endpoint)
}

/// Custom certificate verifier that skips verification (for self-signed certs in P2P)
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

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

/// Receive a single file from the stream
async fn receive_file(
    stream: &mut quinn::RecvStream,
    download_dir: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] receive_file: Starting to read metadata length...".to_string(),
        ))
        .await;

    // 1. Read metadata length (4 bytes, big-endian)
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let meta_len = u32::from_be_bytes(len_buf) as usize;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Metadata length = {} bytes",
            meta_len
        )))
        .await;

    // 2. Read metadata JSON
    let mut meta_buf = vec![0u8; meta_len];
    stream.read_exact(&mut meta_buf).await?;
    let file_info: FileInfo = serde_json::from_slice(&meta_buf)?;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: FileInfo parsed - name: {}, size: {}",
            file_info.file_name, file_info.file_size
        )))
        .await;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Receiving: {} ({} bytes)",
            file_info.file_name, file_info.file_size
        )))
        .await;

    // 3. Create download directory if needed
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Creating dir {:?}",
            download_dir
        )))
        .await;
    tokio::fs::create_dir_all(download_dir).await?;

    // 4. Create file
    let file_path = download_dir.join(&file_info.file_name);
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Creating file {:?}",
            file_path
        )))
        .await;
    let mut file = File::create(&file_path).await?;
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] receive_file: File created successfully".to_string(),
        ))
        .await;

    // 5. Receive file data
    let mut received: u64 = 0;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let total = file_info.file_size;
    let start_time = std::time::Instant::now();

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Starting to receive {} bytes...",
            total
        )))
        .await;

    while received < total {
        let to_read = std::cmp::min(BUFFER_SIZE as u64, total - received) as usize;
        let n = stream.read(&mut buffer[..to_read]).await?.unwrap_or(0);
        if n == 0 {
            let _ = event_tx
                .send(AppEvent::Status(format!(
                    "[DEBUG] receive_file: Stream returned 0 bytes. Received {}/{} bytes",
                    received, total
                )))
                .await;
            break;
        }
        file.write_all(&buffer[..n]).await?;
        received += n as u64;

        // Report progress (less frequent to avoid log spam)
        if received == total || received % (BUFFER_SIZE as u64 * 10) == 0 {
            let progress = (received as f32 / total as f32) * 100.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_bps = if elapsed > 0.0 {
                received as f64 / elapsed
            } else {
                0.0
            };
            let speed = if speed_bps > 1_000_000.0 {
                format!("{:.2} MB/s", speed_bps / 1_000_000.0)
            } else {
                format!("{:.1} KB/s", speed_bps / 1_000.0)
            };
            let _ = event_tx
                .send(AppEvent::TransferProgress {
                    file_name: file_info.file_name.clone(),
                    progress,
                    speed,
                    is_sending: false,
                })
                .await;
        }
    }

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] receive_file: Loop finished. Received {}/{} bytes",
            received, total
        )))
        .await;

    file.flush().await?;

    let _ = event_tx
        .send(AppEvent::TransferCompleted(file_info.file_name.clone()))
        .await;

    Ok(())
}

/// Send files to a remote peer
pub async fn send_files(
    endpoint: &Endpoint,
    target_addr: SocketAddr,
    files: Vec<PathBuf>,
    event_tx: mpsc::Sender<AppEvent>,
    my_peer_id: String,
    my_name: String,
    target_peer_name: String,
    input_code_rx: Option<tokio::sync::oneshot::Receiver<String>>,
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] send_files called. Target: {}, Files: {:?}",
            target_addr, files
        )))
        .await;
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Connecting to: {} ({})",
            target_peer_name, target_addr
        )))
        .await;

    // Connect to peer
    let connection = endpoint.connect(target_addr, "localhost")?.await?;

    // Perform verification handshake
    let (mut send_stream, mut recv_stream) = connection.open_bi().await?;
    if let Err(e) = perform_verification_handshake(
        &mut send_stream,
        &mut recv_stream,
        &event_tx,
        my_peer_id,
        my_name,
        target_peer_name.clone(),
        target_addr,
        input_code_rx,
    )
    .await
    {
        return Err(anyhow!("Handshake failed: {}", e));
    }

    let _ = event_tx
        .send(AppEvent::Status(
            "Connected and verified. Starting file transfer...".to_string(),
        ))
        .await;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] Starting file transfer. Total files: {}",
            files.len()
        )))
        .await;
    for (idx, file_path) in files.iter().enumerate() {
        let _ = event_tx
            .send(AppEvent::Status(format!(
                "[DEBUG] Sending file {}/{}: {}",
                idx + 1,
                files.len(),
                file_path.display()
            )))
            .await;
        if let Err(e) = send_single_file(&connection, file_path, &event_tx).await {
            let _ = event_tx
                .send(AppEvent::Error(format!(
                    "Error sending {}: {}",
                    file_path.display(),
                    e
                )))
                .await;
        } else {
            let _ = event_tx
                .send(AppEvent::Status(format!(
                    "[DEBUG] File sent successfully: {}",
                    file_path.display()
                )))
                .await;
        }
    }

    Ok(())
}

/// Perform verification handshake on sender side
async fn perform_verification_handshake(
    send: &mut quinn::SendStream,
    recv: &mut quinn::RecvStream,
    event_tx: &mpsc::Sender<AppEvent>,
    my_peer_id: String,
    my_name: String,
    target_peer_name: String,
    target_addr: SocketAddr,
    input_code_rx: Option<tokio::sync::oneshot::Receiver<String>>,
) -> Result<()> {
    // 1. Send PairingRequest
    send_msg(
        send,
        &TransferMsg::PairingRequest {
            peer_id: my_peer_id,
            peer_name: my_name,
        },
    )
    .await?;

    // 2. Wait for response
    let msg = recv_msg(recv).await?;
    match msg {
        TransferMsg::PairingAccepted => {
            // Already paired
            let _ = event_tx
                .send(AppEvent::PairingResult {
                    success: true,
                    peer_name: target_peer_name,
                    message: "Already paired.".to_string(),
                })
                .await;
            Ok(())
        }
        TransferMsg::VerificationRequired => {
            // Need verification
            let _ = event_tx
                .send(AppEvent::RequestVerificationCode {
                    target_ip: target_addr.ip().to_string(),
                })
                .await;

            let _ = event_tx
                .send(AppEvent::Status(
                    "Vui lòng nhập mã xác thực trên máy kia...".to_string(),
                ))
                .await;

            // Wait for user input from GUI via channel
            let code = if let Some(rx) = input_code_rx {
                match rx.await {
                    Ok(c) => c,
                    Err(_) => return Err(anyhow!("User cancelled verification input")),
                }
            } else {
                return Err(anyhow!("No input channel provided for verification code"));
            };

            send_msg(send, &TransferMsg::VerificationCode { code }).await?;

            // 3. Wait for final result
            let result_msg = recv_msg(recv).await?;
            match result_msg {
                TransferMsg::VerificationSuccess => {
                    let _ = event_tx
                        .send(AppEvent::PairingResult {
                            success: true,
                            peer_name: target_peer_name,
                            message: "Xác thực thành công".to_string(),
                        })
                        .await;
                    Ok(())
                }
                TransferMsg::VerificationFailed { message } => {
                    let _ = event_tx
                        .send(AppEvent::PairingResult {
                            success: false,
                            peer_name: target_peer_name,
                            message: message.clone(),
                        })
                        .await;
                    Err(anyhow!("Verification failed: {}", message))
                }
                _ => Err(anyhow!("Unexpected response: {:?}", result_msg)),
            }
        }
        _ => Err(anyhow!("Unexpected handshake response: {:?}", msg)),
    }
}

/// Send a single file through the connection
async fn send_single_file(
    connection: &quinn::Connection,
    file_path: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] send_single_file: Opening file {}",
            file_path.display()
        )))
        .await;

    // Open file
    let mut file = File::open(file_path).await?;
    let metadata = file.metadata().await?;
    let file_size = metadata.len();
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Invalid file name"))?
        .to_string();

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Sending: {} ({} bytes)",
            file_name, file_size
        )))
        .await;

    // Open uni-directional stream
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] Opening uni-directional stream...".to_string(),
        ))
        .await;
    let mut send_stream = connection.open_uni().await?;
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] Uni-directional stream opened.".to_string(),
        ))
        .await;

    // 1. Send metadata
    let file_info = FileInfo {
        file_name: file_name.clone(),
        file_size,
        file_path: PathBuf::new(), // Not needed for transfer
    };
    let meta_json = serde_json::to_vec(&file_info)?;
    let meta_len = (meta_json.len() as u32).to_be_bytes();

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] Sending metadata: {} bytes",
            meta_json.len()
        )))
        .await;

    send_stream.write_all(&meta_len).await?;
    send_stream.write_all(&meta_json).await?;

    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] Metadata sent successfully.".to_string(),
        ))
        .await;

    // 2. Send file data
    let mut sent: u64 = 0;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let start_time = std::time::Instant::now();

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "[DEBUG] Starting to send file data: {} bytes",
            file_size
        )))
        .await;

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            let _ = event_tx
                .send(AppEvent::Status(format!(
                    "[DEBUG] File read complete. Total sent: {} bytes",
                    sent
                )))
                .await;
            break;
        }
        send_stream.write_all(&buffer[..n]).await?;
        sent += n as u64;

        // Report progress (less frequent)
        if sent == file_size || sent % (BUFFER_SIZE as u64 * 10) == 0 {
            let progress = (sent as f32 / file_size as f32) * 100.0;
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_bps = if elapsed > 0.0 {
                sent as f64 / elapsed
            } else {
                0.0
            };
            let speed = if speed_bps > 1_000_000.0 {
                format!("{:.2} MB/s", speed_bps / 1_000_000.0)
            } else {
                format!("{:.1} KB/s", speed_bps / 1_000.0)
            };
            let _ = event_tx
                .send(AppEvent::TransferProgress {
                    file_name: file_name.clone(),
                    progress,
                    speed,
                    is_sending: true,
                })
                .await;
        }
    }

    // Finish stream
    let _ = event_tx
        .send(AppEvent::Status("[DEBUG] Finishing stream...".to_string()))
        .await;
    send_stream.finish()?;

    // Wait a short time for data to be flushed
    // We use a timeout instead of stopped().await because receiver might not send STOP_SENDING
    let _ = event_tx
        .send(AppEvent::Status(
            "[DEBUG] Waiting for data to flush (max 2s)...".to_string(),
        ))
        .await;

    // Use tokio timeout to avoid blocking forever
    let _ = tokio::time::timeout(Duration::from_secs(2), send_stream.stopped()).await;

    let _ = event_tx
        .send(AppEvent::Status("[DEBUG] Stream finished.".to_string()))
        .await;

    let _ = event_tx.send(AppEvent::TransferCompleted(file_name)).await;

    Ok(())
}
