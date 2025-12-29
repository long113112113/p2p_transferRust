use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub mod config;
pub mod discovery;
pub mod http_share;
pub mod identity;
pub mod pairing;
pub mod transfer;

use discovery::{DISCOVERY_INTERVAL_SECS, DISCOVERY_PORT, DiscoveryService};
use transfer::{TRANSFER_PORT, make_client_endpoint, make_server_endpoint};

/// Magic bytes to identify our app's packets (6 bytes: "P2PLT\0")
pub const MAGIC_BYTES: &[u8] = b"P2PLT\x00";

#[derive(Debug, Serialize, Deserialize)]
pub enum DiscoveryMsg {
    DiscoveryRequest {
        endpoint_id: String,
        my_name: String,
        port: u16,
    },
    DiscoveryResponse {
        endpoint_id: String,
        my_name: String,
        port: u16,
    },
}

//Struct File metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub file_name: String,
    pub file_size: u64,
    ///Skip file path when serializing
    #[serde(skip)]
    pub file_path: PathBuf,
    /// BLAKE3 hash for integrity verification (64-character hex string)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
}

//Struct command from GUI to Core
#[derive(Debug, Clone)]
pub enum AppCommand {
    ///Broadcast LAN
    StartDiscovery,
    ///Send file to specific IP and list of files
    SendFile {
        target_ip: String,
        target_endpoint_id: String,
        target_peer_name: String,
        files: Vec<PathBuf>,
    },
    ///Cancel transfer
    CancelTransfer,
    /// User submitted verification code (sender side)
    SubmitVerificationCode { target_ip: String, code: String },
    /// Start the HTTP server for file sharing
    StartHttpServer,
    /// Stop the HTTP server
    StopHttpServer,
    /// Respond to upload request from web
    RespondUploadRequest { request_id: String, accepted: bool },
    /// Connect to a remote peer over WAN using Iroh
    WanConnect { target_endpoint_id: String },
    /// Start bore tunnel for WAN HTTP share
    StartWanShare,
    /// Stop bore tunnel
    StopWanShare,
}
//Struct report from Core to GUI
#[derive(Debug, Clone)]
pub enum AppEvent {
    Status(String),

    PeerFound {
        endpoint_id: String,
        ip: String,
        hostname: String,
    },

    TransferProgress {
        file_name: String,
        progress: f32,
        speed: String,
        speed_bps: f64,
        is_sending: bool,
    },
    TransferCompleted(String),
    Error(String),

    /// Receiver: Show this code to user for verification
    ShowVerificationCode {
        code: String,
        from_ip: String,
        from_name: String,
    },

    /// Sender: Ask user to input verification code
    RequestVerificationCode {
        target_ip: String,
    },

    /// Verification/Pairing result
    PairingResult {
        success: bool,
        peer_name: String,
        message: String,
    },

    /// File verification started
    VerificationStarted {
        file_name: String,
        is_sending: bool,
    },

    /// File verification completed
    VerificationCompleted {
        file_name: String,
        is_sending: bool,
        verified: bool,
    },

    /// HTTP share URL is ready (sent once at startup)
    ShareUrlReady {
        url: String,
    },

    /// HTTP server has been started
    HttpServerStarted {
        url: String,
    },

    /// HTTP server has been stopped
    HttpServerStopped,

    /// Upload request from web client
    UploadRequest {
        request_id: String,
        file_name: String,
        file_size: u64,
        from_ip: String,
    },

    /// Upload request cancelled (timeout or client disconnected)
    UploadRequestCancelled {
        request_id: String,
    },

    /// Upload progress update
    UploadProgress {
        request_id: String,
        received_bytes: u64,
        total_bytes: u64,
    },

    /// Upload completed successfully
    UploadCompleted {
        file_name: String,
        saved_path: String,
    },

    /// WAN Connection established
    WanConnected(iroh::endpoint::Connection),

    /// WAN Connection info update (type changed or periodic update)
    WanConnectionInfo {
        connection_type: String,
        rtt_ms: Option<u64>,
    },

    WanShareReady {
        url: String,
    },
    WanShareStopped,
    WanShareError(String),
}

pub async fn run_backend(mut cmd_rx: mpsc::Receiver<AppCommand>, event_tx: mpsc::Sender<AppEvent>) {
    // Load environment variables from .env file (for NGROK_AUTHTOKEN etc.)
    let _ = dotenvy::dotenv();

    // Install rustls crypto provider (required for rustls 0.23+)
    let _ = rustls::crypto::ring::default_provider().install_default();

    // 1. Get Endpoint ID and Hostname (using Iroh NodeId for unified identity)
    let my_endpoint_id = identity::get_iroh_endpoint_id();
    let my_name = hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "Unknown-PC".to_string());

    // Store pending verification channels (IP -> Sender)
    let mut verification_pending: HashMap<String, oneshot::Sender<String>> = HashMap::new();

    // 2. Setup Ports - use constants from discovery module

    // Send message to GUI
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "Endpoint ID: {}, Name: {}",
            my_endpoint_id, my_name
        )))
        .await;

    // 3. Init Discovery Service
    let discovery_service = match DiscoveryService::new(DISCOVERY_PORT).await {
        Ok(ds) => Arc::new(ds),
        Err(e) => {
            tracing::error!("Failed to bind discovery port {}: {}", DISCOVERY_PORT, e);
            let _ = event_tx
                .send(AppEvent::Error(format!(
                    "Cant bind port {}: {}",
                    DISCOVERY_PORT, e
                )))
                .await;
            return;
        }
    };

    // 4. Init QUIC Server Endpoint
    let server_addr: SocketAddr = format!("0.0.0.0:{}", TRANSFER_PORT).parse().unwrap();
    let server_endpoint = match make_server_endpoint(server_addr) {
        Ok(ep) => ep,
        Err(e) => {
            let _ = event_tx
                .send(AppEvent::Error(format!("Cant init QUIC server: {}", e)))
                .await;
            return;
        }
    };
    let _ = event_tx
        .send(AppEvent::Status(format!(
            "QUIC Server listening at port {}",
            TRANSFER_PORT
        )))
        .await;

    // 5. Init QUIC Client Endpoint
    let client_endpoint = match make_client_endpoint() {
        Ok(ep) => Arc::new(ep),
        Err(e) => {
            let _ = event_tx
                .send(AppEvent::Error(format!("Cant init QUIC client: {}", e)))
                .await;
            return;
        }
    };

    // 6. Start QUIC Server Loop
    let download_dir = config::get_download_dir();
    let server_event_tx = event_tx.clone();
    tokio::spawn(async move {
        transfer::run_server(server_endpoint, server_event_tx, download_dir).await;
    });

    // 7. Start Discovery Listening Loop
    discovery_service.start_listening(
        event_tx.clone(),
        my_endpoint_id.clone(),
        my_name.clone(),
        TRANSFER_PORT,
    );

    // 8. Automatic Discovery Loop (Broadcast every 5 seconds)
    let ds_clone = discovery_service.clone();
    let endpoint_id_clone = my_endpoint_id.clone();
    let name_clone = my_name.clone();
    tokio::spawn(async move {
        // Broadcast immediately on start
        ds_clone
            .send_discovery_request(endpoint_id_clone.clone(), name_clone.clone(), TRANSFER_PORT)
            .await;

        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(DISCOVERY_INTERVAL_SECS));
        loop {
            interval.tick().await;
            ds_clone
                .send_discovery_request(
                    endpoint_id_clone.clone(),
                    name_clone.clone(),
                    TRANSFER_PORT,
                )
                .await;
        }
    });

    // 9. HTTP Server state
    let mut http_cancel_token: Option<CancellationToken> = None;
    let upload_state = Arc::new(http_share::UploadState::new());

    // 10. WAN Share (ngrok tunnel) state
    let mut ngrok_tunnel: Option<http_share::NgrokTunnel> = None;
    let mut current_session_token: Option<String> = None;

    // Main loop: Wait for commands from UI
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AppCommand::StartDiscovery => {
                // Trigger manual discovery immediately
                let _ = event_tx
                    .send(AppEvent::Status("Manual scanning...".to_string()))
                    .await;
                discovery_service
                    .send_discovery_request(my_endpoint_id.clone(), my_name.clone(), TRANSFER_PORT)
                    .await;
            }
            AppCommand::SendFile {
                target_ip,
                target_endpoint_id: _target_endpoint_id,
                target_peer_name,
                files,
            } => {
                tracing::info!(
                    "Initiating transfer to {} ({}) with {} files",
                    target_peer_name,
                    target_ip,
                    files.len()
                );
                let target_addr: SocketAddr =
                    match format!("{}:{}", target_ip, TRANSFER_PORT).parse() {
                        Ok(addr) => addr,
                        Err(e) => {
                            let _ = event_tx
                                .send(AppEvent::Error(format!("Invalid address: {}", e)))
                                .await;
                            continue;
                        }
                    };

                // Create channel for verification code
                let (code_tx, code_rx) = oneshot::channel();

                // Store tx in map, keyed by IP
                verification_pending.insert(target_ip.clone(), code_tx);

                let client_endpoint = client_endpoint.clone();
                let evt = event_tx.clone();

                // Create transfer context
                let context = transfer::TransferContext {
                    my_endpoint_id: my_endpoint_id.clone(),
                    my_name: my_name.clone(),
                    target_peer_name,
                };

                tokio::spawn(async move {
                    if let Err(e) = transfer::sender::send_files(
                        &client_endpoint,
                        target_addr,
                        files,
                        evt.clone(),
                        context,
                        Some(code_rx),
                    )
                    .await
                    {
                        let _ = evt
                            .send(AppEvent::Error(format!("File transfer failed: {}", e)))
                            .await;
                    }
                });
            }
            AppCommand::CancelTransfer => {
                let _ = event_tx
                    .send(AppEvent::Status("Task cancelled.".to_string()))
                    .await;
            }
            AppCommand::SubmitVerificationCode { target_ip, code } => {
                if let Some(tx) = verification_pending.remove(&target_ip) {
                    if tx.send(code.clone()).is_err() {
                        let _ = event_tx
                            .send(AppEvent::Error(
                                "Cannot send verification code (task closed)".to_string(),
                            ))
                            .await;
                    } else {
                        let _ = event_tx
                            .send(AppEvent::Status(format!(
                                "Verification code sent to {}",
                                target_ip
                            )))
                            .await;
                    }
                } else {
                    let _ = event_tx
                        .send(AppEvent::Error(format!(
                            "No pending verification session found for {}",
                            target_ip
                        )))
                        .await;
                }
            }
            AppCommand::RespondUploadRequest {
                request_id,
                accepted,
            } => {
                http_share::respond_to_upload(&upload_state, &request_id, accepted).await;
            }
            AppCommand::StartHttpServer => {
                // Stop existing server if running
                if let Some(ct) = http_cancel_token.take() {
                    ct.cancel();
                    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                }

                // Generate new session token and start server
                let session_token = http_share::generate_session_token();
                // Get local IP, preferring non-loopback IPv4
                // Get local IP, prioritizing LAN ranges (192.168.x.x, 10.x.x.x, 172.16.x.x)
                let local_ip = local_ip_address::list_afinet_netifas()
                    .ok()
                    .and_then(|ips| {
                        let mut best_ip = None;
                        for (_name, ip) in ips {
                            if ip.is_loopback() || !ip.is_ipv4() {
                                continue;
                            }
                            let ip_str = ip.to_string();
                            if ip_str.starts_with("192.168.") {
                                return Some(ip_str); // Best match
                            }
                            if ip_str.starts_with("10.") {
                                best_ip = Some(ip_str);
                                continue;
                            }
                            if ip_str.starts_with("172.") && best_ip.is_none() {
                                best_ip = Some(ip_str);
                                continue;
                            }
                            if best_ip.is_none() {
                                best_ip = Some(ip_str);
                            }
                        }
                        best_ip
                    })
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                let share_url = format!(
                    "http://{}:{}/{}",
                    local_ip,
                    http_share::HTTP_PORT,
                    session_token
                );

                let cancel_token = CancellationToken::new();
                http_cancel_token = Some(cancel_token.clone());
                current_session_token = Some(session_token.clone());

                let http_event_tx = event_tx.clone();
                let token_clone = session_token.clone();
                let url_clone = share_url.clone();
                let upload_state_clone = upload_state.clone();

                tokio::spawn(async move {
                    if let Err(e) = http_share::start_default_http_server_with_websocket(
                        &token_clone,
                        http_event_tx.clone(),
                        upload_state_clone,
                        Some(cancel_token),
                    )
                    .await
                    {
                        tracing::error!("HTTP server error: {}", e);
                        let _ = http_event_tx
                            .send(AppEvent::Error(format!("HTTP server failed: {}", e)))
                            .await;
                    }
                });

                // Notify GUI that server started
                let _ = event_tx
                    .send(AppEvent::HttpServerStarted { url: share_url })
                    .await;
                tracing::info!("HTTP server started: {}", url_clone);
            }
            AppCommand::StopHttpServer => {
                if let Some(ct) = http_cancel_token.take() {
                    ct.cancel();
                    let _ = event_tx.send(AppEvent::HttpServerStopped).await;
                    tracing::info!("HTTP server stopped");
                } else {
                    let _ = event_tx
                        .send(AppEvent::Status("HTTP server is not running".to_string()))
                        .await;
                }
            }
            AppCommand::WanConnect { target_endpoint_id } => {
                tracing::info!("=== WAN Connect Command Received ===");
                tracing::info!("Target Endpoint ID: {}", target_endpoint_id);

                // Note: Actual WAN connection is handled in p2p_gui layer
                // which has access to p2p_wan crate
                let _ = event_tx
                    .send(AppEvent::Status(format!(
                        "WAN Connect request: {}",
                        target_endpoint_id
                    )))
                    .await;
            }
            AppCommand::StartWanShare => {
                // First ensure HTTP server is running
                if http_cancel_token.is_none() {
                    // Start HTTP server first
                    let session_token = http_share::generate_session_token();
                    let local_ip = local_ip_address::list_afinet_netifas()
                        .ok()
                        .and_then(|ips| {
                            let mut best_ip = None;
                            for (_name, ip) in ips {
                                if ip.is_loopback() || !ip.is_ipv4() {
                                    continue;
                                }
                                let ip_str = ip.to_string();
                                if ip_str.starts_with("192.168.") {
                                    return Some(ip_str);
                                }
                                if ip_str.starts_with("10.") {
                                    best_ip = Some(ip_str);
                                    continue;
                                }
                                if ip_str.starts_with("172.") && best_ip.is_none() {
                                    best_ip = Some(ip_str);
                                    continue;
                                }
                                if best_ip.is_none() {
                                    best_ip = Some(ip_str);
                                }
                            }
                            best_ip
                        })
                        .unwrap_or_else(|| "127.0.0.1".to_string());
                    let share_url = format!(
                        "http://{}:{}/{}",
                        local_ip,
                        http_share::HTTP_PORT,
                        session_token
                    );

                    let cancel_token = CancellationToken::new();
                    http_cancel_token = Some(cancel_token.clone());
                    current_session_token = Some(session_token.clone());

                    let http_event_tx = event_tx.clone();
                    let token_clone = session_token.clone();
                    let upload_state_clone = upload_state.clone();

                    tokio::spawn(async move {
                        if let Err(e) = http_share::start_default_http_server_with_websocket(
                            &token_clone,
                            http_event_tx.clone(),
                            upload_state_clone,
                            Some(cancel_token),
                        )
                        .await
                        {
                            tracing::error!("HTTP server error: {}", e);
                            let _ = http_event_tx
                                .send(AppEvent::Error(format!("HTTP server failed: {}", e)))
                                .await;
                        }
                    });

                    let _ = event_tx
                        .send(AppEvent::HttpServerStarted { url: share_url })
                        .await;
                }

                // Now start ngrok tunnel
                let session_token = current_session_token.clone().unwrap_or_default();
                let evt = event_tx.clone();

                match http_share::NgrokTunnel::start(http_share::HTTP_PORT, &session_token).await {
                    Ok(tunnel) => {
                        let public_url = tunnel.public_url().to_string();
                        ngrok_tunnel = Some(tunnel);
                        let _ = evt.send(AppEvent::WanShareReady { url: public_url }).await;
                    }
                    Err(e) => {
                        tracing::error!("Failed to start ngrok tunnel: {}", e);
                        let _ = evt
                            .send(AppEvent::WanShareError(format!(
                                "Failed to start tunnel: {}",
                                e
                            )))
                            .await;
                    }
                }
            }
            AppCommand::StopWanShare => {
                if let Some(tunnel) = ngrok_tunnel.take() {
                    tunnel.stop();
                    let _ = event_tx.send(AppEvent::WanShareStopped).await;
                    tracing::info!("WAN share tunnel stopped");
                } else {
                    let _ = event_tx
                        .send(AppEvent::Status("WAN share is not running".to_string()))
                        .await;
                }
            }
        }
    }
}
