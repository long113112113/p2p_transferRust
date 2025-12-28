use crate::{AppEvent, DiscoveryMsg, MAGIC_BYTES};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

/// Default UDP port for peer discovery
pub const DISCOVERY_PORT: u16 = 8888;

/// Buffer size for receiving discovery packets
const DISCOVERY_BUFFER_SIZE: usize = 2048;

/// Broadcast address for LAN discovery
const BROADCAST_ADDR: &str = "255.255.255.255";

/// Interval between automatic discovery broadcasts (seconds)
pub const DISCOVERY_INTERVAL_SECS: u64 = 5;

/// Build a discovery packet with magic bytes prefix
fn build_packet(msg: &DiscoveryMsg) -> Option<Vec<u8>> {
    serde_json::to_vec(msg).ok().map(|json_bytes| {
        let mut packet = MAGIC_BYTES.to_vec();
        packet.extend_from_slice(&json_bytes);
        packet
    })
}

pub struct DiscoveryService {
    socket: Arc<UdpSocket>,
}

impl DiscoveryService {
    pub async fn new(port: u16) -> Result<Self, std::io::Error> {
        // Bind to 0.0.0.0 to listen on all interfaces
        let addr = SocketAddr::from(([0, 0, 0, 0], port));
        let socket = UdpSocket::bind(addr).await?;

        // Enable broadcast
        socket.set_broadcast(true)?;

        Ok(Self {
            socket: Arc::new(socket),
        })
    }

    /// Broadcast to local
    pub async fn send_discovery_request(&self, endpoint_id: String, my_name: String, port: u16) {
        let msg = DiscoveryMsg::DiscoveryRequest {
            endpoint_id,
            my_name,
            port,
        };
        if let Some(packet) = build_packet(&msg) {
            let broadcast_addr = format!("{}:{}", BROADCAST_ADDR, DISCOVERY_PORT);
            let _ = self.socket.send_to(&packet, broadcast_addr).await;
        }
    }

    /// Reply directly to a specific peer
    pub async fn send_discovery_response(
        &self,
        target: SocketAddr,
        endpoint_id: String,
        my_name: String,
        port: u16,
    ) {
        let msg = DiscoveryMsg::DiscoveryResponse {
            endpoint_id,
            my_name,
            port,
        };
        if let Some(packet) = build_packet(&msg) {
            let _ = self.socket.send_to(&packet, target).await;
        }
    }

    /// Start listening loop
    pub fn start_listening(
        &self,
        event_tx: mpsc::Sender<AppEvent>,
        my_endpoint_id: String,
        my_name: String,
        my_port: u16,
    ) {
        let socket = self.socket.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; DISCOVERY_BUFFER_SIZE];
            while let Ok((len, addr)) = socket.recv_from(&mut buf).await {
                // Check identify packet
                if len < MAGIC_BYTES.len() || &buf[..MAGIC_BYTES.len()] != MAGIC_BYTES {
                    //ignore
                    continue;
                }

                // Extract JSON data after identify bytes
                let data = &buf[MAGIC_BYTES.len()..len];

                if let Ok(msg) = serde_json::from_slice::<DiscoveryMsg>(data) {
                    match msg {
                        DiscoveryMsg::DiscoveryRequest {
                            endpoint_id: remote_endpoint_id,
                            my_name: remote_name,
                            port: _remote_port,
                        } => {
                            //ignore self
                            if remote_endpoint_id != my_endpoint_id {
                                let response_msg = DiscoveryMsg::DiscoveryResponse {
                                    endpoint_id: my_endpoint_id.clone(),
                                    my_name: my_name.clone(),
                                    port: my_port,
                                };
                                if let Some(packet) = build_packet(&response_msg) {
                                    let _ = socket.send_to(&packet, addr).await;
                                }

                                //treat this as "Peer found" immediately
                                let _ = event_tx
                                    .send(AppEvent::PeerFound {
                                        endpoint_id: remote_endpoint_id,
                                        ip: addr.ip().to_string(),
                                        hostname: remote_name,
                                    })
                                    .await;
                            }
                        }
                        DiscoveryMsg::DiscoveryResponse {
                            endpoint_id: remote_endpoint_id,
                            my_name: remote_name,
                            ..
                        } => {
                            //Found a peer
                            if remote_endpoint_id != my_endpoint_id {
                                let _ = event_tx
                                    .send(AppEvent::PeerFound {
                                        endpoint_id: remote_endpoint_id,
                                        ip: addr.ip().to_string(),
                                        hostname: remote_name,
                                    })
                                    .await;
                            }
                        }
                    }
                }
            }
        });
    }
}
