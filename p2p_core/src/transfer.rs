//! QUIC-based file transfer module using quinn.
//!
//! This module provides:
//! - Self-signed certificate generation
//! - QUIC server endpoint (to receive files)
//! - QUIC client endpoint (to send files)

use crate::{AppEvent, FileInfo};
use anyhow::{Result, anyhow};
use quinn::{ClientConfig, Endpoint, ServerConfig, TransportConfig};
use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
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

/// Generate a self-signed certificate for QUIC
pub fn generate_self_signed_cert()
-> Result<(Vec<CertificateDer<'static>>, PrivatePkcs8KeyDer<'static>)> {
    let certified_key = generate_simple_self_signed(vec!["localhost".to_string()])?;
    let key = PrivatePkcs8KeyDer::from(certified_key.signing_key.serialize_der());
    let cert_der = CertificateDer::from(certified_key.cert.der().to_vec());
    Ok((vec![cert_der], key))
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
    let crypto = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

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
    while let Some(incoming) = endpoint.accept().await {
        let event_tx = event_tx.clone();
        let download_dir = download_dir.clone();

        tokio::spawn(async move {
            match incoming.await {
                Ok(connection) => {
                    let remote_addr = connection.remote_address();
                    let _ = event_tx
                        .send(AppEvent::Status(format!("Kết nối từ: {}", remote_addr)))
                        .await;

                    // Accept uni-directional stream for file data
                    while let Ok(mut recv_stream) = connection.accept_uni().await {
                        let event_tx = event_tx.clone();
                        let download_dir = download_dir.clone();

                        tokio::spawn(async move {
                            if let Err(e) =
                                receive_file(&mut recv_stream, &download_dir, &event_tx).await
                            {
                                let _ = event_tx
                                    .send(AppEvent::Error(format!("Lỗi nhận file: {}", e)))
                                    .await;
                            }
                        });
                    }
                }
                Err(e) => {
                    let _ = event_tx
                        .send(AppEvent::Error(format!("Lỗi kết nối: {}", e)))
                        .await;
                }
            }
        });
    }
}

/// Receive a single file from the stream
async fn receive_file(
    stream: &mut quinn::RecvStream,
    download_dir: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
    // 1. Read metadata length (4 bytes, big-endian)
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let meta_len = u32::from_be_bytes(len_buf) as usize;

    // 2. Read metadata JSON
    let mut meta_buf = vec![0u8; meta_len];
    stream.read_exact(&mut meta_buf).await?;
    let file_info: FileInfo = serde_json::from_slice(&meta_buf)?;

    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Đang nhận: {} ({} bytes)",
            file_info.file_name, file_info.file_size
        )))
        .await;

    // 3. Create download directory if needed
    tokio::fs::create_dir_all(download_dir).await?;

    // 4. Create file
    let file_path = download_dir.join(&file_info.file_name);
    let mut file = File::create(&file_path).await?;

    // 5. Receive file data
    let mut received: u64 = 0;
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let total = file_info.file_size;

    while received < total {
        let to_read = std::cmp::min(BUFFER_SIZE as u64, total - received) as usize;
        let n = stream.read(&mut buffer[..to_read]).await?.unwrap_or(0);
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n]).await?;
        received += n as u64;

        // Report progress
        let progress = (received as f32 / total as f32) * 100.0;
        let speed = format!("{:.1} KB/s", (received as f64 / 1024.0)); // Simplified
        let _ = event_tx
            .send(AppEvent::TransferProgress {
                file_name: file_info.file_name.clone(),
                progress,
                speed,
            })
            .await;
    }

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
) -> Result<()> {
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Đang kết nối tới: {}",
            target_addr
        )))
        .await;

    // Connect to peer
    let connection = endpoint.connect(target_addr, "localhost")?.await?;

    let _ = event_tx
        .send(AppEvent::Status(
            "Đã kết nối. Bắt đầu gửi file...".to_string(),
        ))
        .await;

    for file_path in files {
        if let Err(e) = send_single_file(&connection, &file_path, &event_tx).await {
            let _ = event_tx
                .send(AppEvent::Error(format!(
                    "Lỗi gửi {}: {}",
                    file_path.display(),
                    e
                )))
                .await;
        }
    }

    Ok(())
}

/// Send a single file through the connection
async fn send_single_file(
    connection: &quinn::Connection,
    file_path: &PathBuf,
    event_tx: &mpsc::Sender<AppEvent>,
) -> Result<()> {
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
            "Đang gửi: {} ({} bytes)",
            file_name, file_size
        )))
        .await;

    // Open uni-directional stream
    let mut send_stream = connection.open_uni().await?;

    // 1. Send metadata
    let file_info = FileInfo {
        file_name: file_name.clone(),
        file_size,
        file_path: PathBuf::new(), // Not needed for transfer
    };
    let meta_json = serde_json::to_vec(&file_info)?;
    let meta_len = (meta_json.len() as u32).to_be_bytes();
    send_stream.write_all(&meta_len).await?;
    send_stream.write_all(&meta_json).await?;

    // 2. Send file data
    let mut sent: u64 = 0;
    let mut buffer = vec![0u8; BUFFER_SIZE];

    loop {
        let n = file.read(&mut buffer).await?;
        if n == 0 {
            break;
        }
        send_stream.write_all(&buffer[..n]).await?;
        sent += n as u64;

        // Report progress
        let progress = (sent as f32 / file_size as f32) * 100.0;
        let speed = format!("{:.1} KB/s", (sent as f64 / 1024.0)); // Simplified
        let _ = event_tx
            .send(AppEvent::TransferProgress {
                file_name: file_name.clone(),
                progress,
                speed,
            })
            .await;
    }

    // Finish stream
    send_stream.finish()?;

    let _ = event_tx.send(AppEvent::TransferCompleted(file_name)).await;

    Ok(())
}
