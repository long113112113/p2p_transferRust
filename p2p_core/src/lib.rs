use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;

pub mod config;
pub mod discovery;
use discovery::DiscoveryService;

/// Magic bytes to identify our app's packets (6 bytes: "P2PLT\0")
pub const MAGIC_BYTES: &[u8] = b"P2PLT\x00";

#[derive(Debug, Serialize, Deserialize)]
pub enum DiscoveryMsg {
    DiscoveryRequest {
        peer_id: String,
        my_name: String,
        tcp_port: u16,
    },
    DiscoveryResponse {
        peer_id: String,
        my_name: String,
        tcp_port: u16,
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
}

//Struct command from GUI to Core
#[derive(Debug, Clone)]
pub enum AppCommand {
    ///Broadcast LAN
    StartDiscovery,
    ///Send file to specific IP and list of files
    SendFile {
        target_ip: String,
        files: Vec<PathBuf>,
    },
    ///Cancel transfer
    CancelTransfer,
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
    },
    TransferCompleted(String),
    Error(String),
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

    // 2. Setup Ports (Hardcoded for now, ideal to be dynamic or config)
    let tcp_port = 9000;
    let discovery_port = 8888;

    // Send message to GUI
    let _ = event_tx.send(AppEvent::Status(format!(
        "Backend khởi động. Tên: {}",
        my_name
    )));

    // 3. Init Discovery Service
    let discovery_service = match DiscoveryService::new(discovery_port).await {
        Ok(ds) => Arc::new(ds),
        Err(e) => {
            let _ = event_tx.send(AppEvent::Error(format!(
                "Không thể bind cổng {}: {}",
                discovery_port, e
            )));
            return;
        }
    };

    // 4. Start Listening Loop
    discovery_service.start_listening(
        event_tx.clone(),
        my_peer_id.clone(),
        my_name.clone(),
        tcp_port,
    );

    // 5. Automatic Discovery Loop (Broadcast every 5 seconds)
    let ds_clone = discovery_service.clone();
    let peer_id_clone = my_peer_id.clone();
    let name_clone = my_name.clone();
    tokio::spawn(async move {
        // Broadcast immediately on start
        ds_clone
            .send_discovery_request(peer_id_clone.clone(), name_clone.clone(), tcp_port)
            .await;

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            ds_clone
                .send_discovery_request(peer_id_clone.clone(), name_clone.clone(), tcp_port)
                .await;
        }
    });

    // Vòng lặp chính: Chờ lệnh từ UI
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AppCommand::StartDiscovery => {
                // Trigger manual discovery immediately
                let _ = event_tx.send(AppEvent::Status("Đang quét thủ công...".to_string()));
                discovery_service
                    .send_discovery_request(my_peer_id.clone(), my_name.clone(), tcp_port)
                    .await;
            }
            AppCommand::SendFile { target_ip, files } => {
                let msg = format!("Đang chuẩn bị gửi {} file tới {}", files.len(), target_ip);
                let _ = event_tx.send(AppEvent::Status(msg));
            }
            AppCommand::CancelTransfer => {
                let _ = event_tx.send(AppEvent::Status("Đã hủy tác vụ.".to_string()));
            }
        }
    }
}
