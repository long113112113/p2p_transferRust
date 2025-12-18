use crate::{AppEvent, DiscoveryMsg, MAGIC_BYTES};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;

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

    /// Broadcast a "Hello I'm looking for peers" message
    pub async fn send_discovery_request(&self, peer_id: String, my_name: String, tcp_port: u16) {
        let msg = DiscoveryMsg::DiscoveryRequest {
            peer_id,
            my_name,
            tcp_port,
        };
        if let Ok(json_bytes) = serde_json::to_vec(&msg) {
            // Prepend MAGIC_BYTES to identify our app's packets
            let mut packet = MAGIC_BYTES.to_vec();
            packet.extend_from_slice(&json_bytes);

            // Broadcast to 255.255.255.255
            let broadcast_addr = "255.255.255.255:8888";
            let _ = self.socket.send_to(&packet, broadcast_addr).await;
        }
    }

    /// Reply directly to a specific peer saying "I am here"
    pub async fn send_discovery_response(
        &self,
        target: SocketAddr,
        peer_id: String,
        my_name: String,
        tcp_port: u16,
    ) {
        let msg = DiscoveryMsg::DiscoveryResponse {
            peer_id,
            my_name,
            tcp_port,
        };
        if let Ok(json_bytes) = serde_json::to_vec(&msg) {
            // Prepend MAGIC_BYTES to identify our app's packets
            let mut packet = MAGIC_BYTES.to_vec();
            packet.extend_from_slice(&json_bytes);

            let _ = self.socket.send_to(&packet, target).await;
        }
    }

    /// Start listening loop
    pub fn start_listening(
        &self,
        event_tx: mpsc::Sender<AppEvent>,
        my_peer_id: String,
        my_name: String,
        my_tcp_port: u16,
    ) {
        let socket = self.socket.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 2048]; // Increased buffer for magic bytes + json
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        // Check for MAGIC_BYTES prefix to filter only our app's packets
                        if len < MAGIC_BYTES.len() || &buf[..MAGIC_BYTES.len()] != MAGIC_BYTES {
                            // Not our packet, ignore silently
                            continue;
                        }

                        // Extract JSON data after magic bytes
                        let data = &buf[MAGIC_BYTES.len()..len];

                        if let Ok(msg) = serde_json::from_slice::<DiscoveryMsg>(data) {
                            match msg {
                                DiscoveryMsg::DiscoveryRequest {
                                    peer_id: remote_peer_id,
                                    my_name: remote_name,
                                    tcp_port: _remote_port,
                                } => {
                                    // Someone is looking for peers.
                                    // Don't respond to self (using peer_id for reliable check)
                                    if remote_peer_id != my_peer_id {
                                        let response_msg = DiscoveryMsg::DiscoveryResponse {
                                            peer_id: my_peer_id.clone(),
                                            my_name: my_name.clone(),
                                            tcp_port: my_tcp_port,
                                        };
                                        if let Ok(json_bytes) = serde_json::to_vec(&response_msg) {
                                            let mut packet = MAGIC_BYTES.to_vec();
                                            packet.extend_from_slice(&json_bytes);
                                            let _ = socket.send_to(&packet, addr).await;
                                        }

                                        // Also treat this as "Peer found" immediately
                                        let _ = event_tx
                                            .send(AppEvent::PeerFound {
                                                peer_id: remote_peer_id,
                                                ip: addr.ip().to_string(),
                                                hostname: remote_name,
                                            })
                                            .await;
                                    }
                                }
                                DiscoveryMsg::DiscoveryResponse {
                                    peer_id: remote_peer_id,
                                    my_name: remote_name,
                                    ..
                                } => {
                                    // Found a peer!
                                    if remote_peer_id != my_peer_id {
                                        let _ = event_tx
                                            .send(AppEvent::PeerFound {
                                                peer_id: remote_peer_id,
                                                ip: addr.ip().to_string(),
                                                hostname: remote_name,
                                            })
                                            .await;
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {
                        // Error reading socket
                        break;
                    }
                }
            }
        });
    }
}
