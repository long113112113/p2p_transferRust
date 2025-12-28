use anyhow::{Context, Result};
use iroh::endpoint::Incoming;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey, Watcher};
use p2p_core::AppEvent;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use crate::protocol::{ALPN, WanTransferMsg, recv_msg, send_msg};
use crate::receiver::receive_file;

/// Manages incoming P2P connections using Iroh
pub struct ConnectionListener {
    endpoint: Endpoint,
    download_dir: PathBuf,
    event_tx: mpsc::Sender<AppEvent>,
}

impl ConnectionListener {
    /// Creates a new listener with the given secret key
    ///
    /// # Arguments
    /// * `secret_key` - The secret key that determines the node's identity
    /// * `download_dir` - Directory to save received files
    /// * `event_tx` - Channel to send events to GUI
    pub async fn new(
        secret_key: SecretKey,
        download_dir: PathBuf,
        event_tx: mpsc::Sender<AppEvent>,
    ) -> Result<Self> {
        info!("Initializing Iroh listener endpoint...");

        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .context("Failed to bind endpoint")?;

        let node_id = endpoint.id();
        info!("Iroh endpoint initialized with Node ID: {}", node_id);

        let local_addr = endpoint.addr();
        info!("Endpoint address: {:?}", local_addr);

        Ok(Self {
            endpoint,
            download_dir,
            event_tx,
        })
    }

    /// Returns the underlying endpoint
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Returns the Node ID of this endpoint
    pub fn node_id(&self) -> EndpointId {
        self.endpoint.id()
    }

    /// Returns the full node address including relay information
    pub fn node_addr(&self) -> EndpointAddr {
        self.endpoint.addr()
    }

    /// Connect to a remote peer using this endpoint
    /// Reuse endpoint/port for both incoming and outgoing connections
    pub async fn connect(&self, node_id: EndpointId) -> Result<iroh::endpoint::Connection> {
        info!(
            "Connecting to peer {} from existing listener endpoint...",
            node_id
        );
        let conn = self.endpoint.connect(node_id, ALPN).await?;
        Ok(conn)
    }

    /// Starts listening for incoming connections
    ///
    /// This function will run indefinitely, accepting and handling connections.
    /// Each connection is handled in a separate task.
    pub async fn listen(&self) -> Result<()> {
        info!("Waiting for incoming connections...");
        info!("Share your Node ID with others: {}", self.node_id());

        loop {
            match self.endpoint.accept().await {
                Some(incoming) => {
                    info!("Incoming connection detected, spawning handler...");

                    let download_dir = self.download_dir.clone();
                    let event_tx = self.event_tx.clone();
                    // Spawn a task to handle this connection
                    tokio::spawn(async move {
                        if let Err(e) =
                            Self::handle_connection(incoming, download_dir, event_tx).await
                        {
                            error!("Error handling connection: {}", e);
                        }
                    });
                }
                None => {
                    warn!("Endpoint closed, stopping listener");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handles an individual incoming connection
    async fn handle_connection(
        incoming: Incoming,
        download_dir: PathBuf,
        event_tx: mpsc::Sender<AppEvent>,
    ) -> Result<()> {
        let connection = incoming.await.context("Failed to accept connection")?;
        let remote_node_id = connection.remote_id();

        info!(
            "Connection accepted and established with: {}",
            remote_node_id
        );

        // Handle multiple file transfers on this connection
        loop {
            // Accept the next bi-directional stream (each file uses a new stream)
            match connection.accept_bi().await {
                Ok((mut send, mut recv)) => {
                    info!("Bi-directional stream opened with: {}", remote_node_id);

                    // Receive the first message which should be FileMetadata
                    match recv_msg(&mut recv).await {
                        Ok(WanTransferMsg::FileMetadata { info }) => {
                            info!(
                                "Receiving file: {} ({} bytes)",
                                info.file_name, info.file_size
                            );

                            if let Err(e) =
                                receive_file(&mut send, &mut recv, &download_dir, &event_tx, info)
                                    .await
                            {
                                error!("Error receiving file: {}", e);
                                let _ = send_msg(
                                    &mut send,
                                    &WanTransferMsg::Error {
                                        message: e.to_string(),
                                    },
                                )
                                .await;
                            }
                        }
                        Ok(msg) => {
                            warn!("Unexpected message: {:?}", msg);
                        }
                        Err(e) => {
                            // Stream closed or error - this is normal when transfer is complete
                            if e.to_string().contains("closed") {
                                info!("Stream closed by peer: {}", remote_node_id);
                            } else {
                                error!("Error reading message: {}", e);
                            }
                            break;
                        }
                    }
                }
                Err(e) => {
                    // Connection closed - this is normal after all transfers complete
                    if e.to_string().contains("closed") {
                        info!("Connection closed by peer: {}", remote_node_id);
                    } else {
                        error!("Failed to accept bi-directional stream: {}", e);
                    }
                    break;
                }
            }
        }
        info!("Connection handler finished for: {}", remote_node_id);
        Ok(())
    }

    /// Gracefully closes the endpoint
    pub async fn close(self) -> Result<()> {
        info!("Closing listener endpoint...");
        self.endpoint.close().await;
        Ok(())
    }
}

/// Monitor connection type (Direct/Relay) and send updates to GUI
///
/// This function polls for connection type changes and periodically
/// reports the current status including RTT.
pub async fn spawn_connection_monitor(
    endpoint: Endpoint,
    peer_id: EndpointId,
    connection: iroh::endpoint::Connection,
    event_tx: mpsc::Sender<AppEvent>,
) {
    info!("Starting connection monitor for peer: {}", peer_id);

    let mut last_type_str = String::new();
    let mut interval = tokio::time::interval(Duration::from_secs(3));

    loop {
        interval.tick().await;

        // Get current connection type
        if let Some(mut watcher) = endpoint.conn_type(peer_id) {
            let conn_type = watcher.get();
            let type_str = format!("{:?}", conn_type);

            // Get RTT from connection
            let rtt = connection.rtt();
            let rtt_ms = Some(rtt.as_millis() as u64);

            // Log when type changes
            if type_str != last_type_str {
                info!("Connection type changed to: {} (RTT: {:?})", type_str, rtt);
                last_type_str = type_str.clone();
            }

            // Format connection type for display
            let display_type = match conn_type {
                iroh::endpoint::ConnectionType::Direct(_) => "Direct ✓".to_string(),
                iroh::endpoint::ConnectionType::Relay(_) => "Relay".to_string(),
                iroh::endpoint::ConnectionType::Mixed(_, _) => "Mixed".to_string(),
                iroh::endpoint::ConnectionType::None => "None".to_string(),
            };

            let _ = event_tx
                .send(AppEvent::WanConnectionInfo {
                    connection_type: display_type,
                    rtt_ms,
                })
                .await;
        } else {
            warn!(
                "Could not get connection type watcher for peer: {}",
                peer_id
            );
            break;
        }
    }
}
