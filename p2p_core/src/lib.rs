use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

pub mod config;
pub mod discovery;
pub mod pairing;
pub mod transfer;

use discovery::{DISCOVERY_INTERVAL_SECS, DISCOVERY_PORT, DiscoveryService};
use transfer::{TRANSFER_PORT, make_client_endpoint, make_server_endpoint};

/// Magic bytes to identify our app's packets (6 bytes: "P2PLT\0")
pub const MAGIC_BYTES: &[u8] = b"P2PLT\x00";

#[derive(Debug, Serialize, Deserialize)]
pub enum DiscoveryMsg {
    DiscoveryRequest {
        peer_id: String,
        my_name: String,
        port: u16,
    },
    DiscoveryResponse {
        peer_id: String,
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
        target_peer_id: String,
        target_peer_name: String,
        files: Vec<PathBuf>,
    },
    ///Cancel transfer
    CancelTransfer,
    /// User submitted verification code (sender side)
    SubmitVerificationCode { target_ip: String, code: String },
}
//Struct report from Core to GUI
#[derive(Debug, Clone)]
pub enum AppEvent {
    Status(String),

    PeerFound {
        peer_id: String,
        ip: String,
        hostname: String,
    },

    TransferProgress {
        file_name: String,
        progress: f32,
        speed: String,
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
}

/// New Thread
/// cmd_rx: listent from GUI
/// event_tx: send to GUI
pub async fn run_backend(mut cmd_rx: mpsc::Receiver<AppCommand>, event_tx: mpsc::Sender<AppEvent>) {
    // 1. Get Peer ID and Hostname
    let my_peer_id = config::get_or_create_peer_id();
    let my_name = hostname::get()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_else(|| "Unknown-PC".to_string());

    // Store pending verification channels (IP -> Sender)
    let mut verification_pending: HashMap<String, oneshot::Sender<String>> = HashMap::new();

    // 2. Setup Ports - use constants from discovery module

    // Send message to GUI
    let _ = event_tx.send(AppEvent::Status(format!(
        "Backend started. Name: {}",
        my_name
    )));

    // 3. Init Discovery Service
    let discovery_service = match DiscoveryService::new(DISCOVERY_PORT).await {
        Ok(ds) => Arc::new(ds),
        Err(e) => {
            let _ = event_tx.send(AppEvent::Error(format!(
                "Cant bind port {}: {}",
                DISCOVERY_PORT, e
            )));
            return;
        }
    };

    // 4. Init QUIC Server Endpoint
    let server_addr: SocketAddr = format!("0.0.0.0:{}", TRANSFER_PORT).parse().unwrap();
    let server_endpoint = match make_server_endpoint(server_addr) {
        Ok(ep) => ep,
        Err(e) => {
            let _ = event_tx.send(AppEvent::Error(format!("Cant init QUIC server: {}", e)));
            return;
        }
    };
    let _ = event_tx.send(AppEvent::Status(format!(
        "QUIC Server listening at port {}",
        TRANSFER_PORT
    )));

    // 5. Init QUIC Client Endpoint
    let client_endpoint = match make_client_endpoint() {
        Ok(ep) => Arc::new(ep),
        Err(e) => {
            let _ = event_tx.send(AppEvent::Error(format!("Cant init QUIC client: {}", e)));
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
        my_peer_id.clone(),
        my_name.clone(),
        TRANSFER_PORT,
    );

    // 8. Automatic Discovery Loop (Broadcast every 5 seconds)
    let ds_clone = discovery_service.clone();
    let peer_id_clone = my_peer_id.clone();
    let name_clone = my_name.clone();
    tokio::spawn(async move {
        // Broadcast immediately on start
        ds_clone
            .send_discovery_request(peer_id_clone.clone(), name_clone.clone(), TRANSFER_PORT)
            .await;

        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(DISCOVERY_INTERVAL_SECS));
        loop {
            interval.tick().await;
            ds_clone
                .send_discovery_request(peer_id_clone.clone(), name_clone.clone(), TRANSFER_PORT)
                .await;
        }
    });

    // Main loop: Wait for commands from UI
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AppCommand::StartDiscovery => {
                // Trigger manual discovery immediately
                let _ = event_tx.send(AppEvent::Status("Manual scanning...".to_string()));
                discovery_service
                    .send_discovery_request(my_peer_id.clone(), my_name.clone(), TRANSFER_PORT)
                    .await;
            }
            AppCommand::SendFile {
                target_ip,
                target_peer_id: _target_peer_id,
                target_peer_name,
                files,
            } => {
                let target_addr: SocketAddr = match format!("{}:{}", target_ip, TRANSFER_PORT)
                    .parse()
                {
                    Ok(addr) => addr,
                    Err(e) => {
                        let _ = event_tx.send(AppEvent::Error(format!("Invalid address: {}", e)));
                        continue;
                    }
                };

                // Create channel for verification code
                let (code_tx, code_rx) = oneshot::channel();

                // Store tx in map, keyed by IP
                // Note: If multiple transfers to same IP, this overwrites.
                // For MVP this is acceptable (assume one active handshake per peer).
                verification_pending.insert(target_ip.clone(), code_tx);

                let client_ep = client_endpoint.clone();
                let event_tx_clone = event_tx.clone();
                let my_peer_id_clone = my_peer_id.clone();
                let my_name_clone = my_name.clone();

                tokio::spawn(async move {
                    if let Err(e) = transfer::send_files(
                        &client_ep,
                        target_addr,
                        files,
                        event_tx_clone.clone(),
                        my_peer_id_clone,
                        my_name_clone,
                        target_peer_name,
                        Some(code_rx),
                    )
                    .await
                    {
                        let _ =
                            event_tx_clone.send(AppEvent::Error(format!("Send file error: {}", e)));
                    }
                });
            }
            AppCommand::CancelTransfer => {
                let _ = event_tx.send(AppEvent::Status("Task cancelled.".to_string()));
                // Also clear any pending verifications?
                // verification_pending.clear(); // Maybe not all
            }
            AppCommand::SubmitVerificationCode { target_ip, code } => {
                if let Some(tx) = verification_pending.remove(&target_ip) {
                    if let Err(_) = tx.send(code.clone()) {
                        let _ = event_tx.send(AppEvent::Error(
                            "Cannot send verification code (task closed)".to_string(),
                        ));
                    } else {
                        let _ = event_tx.send(AppEvent::Status(format!(
                            "Verification code sent to {}",
                            target_ip
                        )));
                    }
                } else {
                    let _ = event_tx.send(AppEvent::Error(format!(
                        "No pending verification session found for {}",
                        target_ip
                    )));
                }
            }
        }
    }
}
