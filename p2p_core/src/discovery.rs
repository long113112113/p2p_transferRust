use crate::{AppEvent, DiscoveryMsg};
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
    pub async fn send_discovery_request(&self, my_name: String, tcp_port: u16) {
        let msg = DiscoveryMsg::DiscoveryRequest { my_name, tcp_port };
        if let Ok(bytes) = serde_json::to_vec(&msg) {
            // Broadcast to 255.255.255.255
            let broadcast_addr = "255.255.255.255:8888";
            // Note: In real world, 255.255.255.255 might not work on some OS/networks properly without specific interface binding.
            // But for simple LAN it usually works.
            // Port 8888 is hardcoded for discovery for now.
            let _ = self.socket.send_to(&bytes, broadcast_addr).await;
        }
    }

    /// Reply directly to a specific peer saying "I am here"
    pub async fn send_discovery_response(
        &self,
        target: SocketAddr,
        my_name: String,
        tcp_port: u16,
    ) {
        let msg = DiscoveryMsg::DiscoveryResponse { my_name, tcp_port };
        if let Ok(bytes) = serde_json::to_vec(&msg) {
            let _ = self.socket.send_to(&bytes, target).await;
        }
    }

    /// Start listening loop
    pub fn start_listening(
        &self,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        my_name: String,
        my_tcp_port: u16,
    ) {
        let socket = self.socket.clone();

        tokio::spawn(async move {
            let mut buf = [0u8; 1024];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((len, addr)) => {
                        // Ignore our own packets if possible?
                        // UDP multicast/broadcast often loops back.
                        // We will filter by checking message content or relying on logic.

                        let data = &buf[..len];
                        if let Ok(msg) = serde_json::from_slice::<DiscoveryMsg>(data) {
                            match msg {
                                DiscoveryMsg::DiscoveryRequest {
                                    my_name: remote_name,
                                    tcp_port: _remote_port,
                                } => {
                                    // Someone is looking for peers.
                                    // If it is NOT me (simple check by name/port comparison or just respond)
                                    // For now we just respond to everyone, client logic can filter self.

                                    // Don't respond to self
                                    if remote_name != my_name {
                                        let response_msg = DiscoveryMsg::DiscoveryResponse {
                                            my_name: my_name.clone(),
                                            tcp_port: my_tcp_port,
                                        };
                                        if let Ok(resp_bytes) = serde_json::to_vec(&response_msg) {
                                            let _ = socket.send_to(&resp_bytes, addr).await;
                                        }

                                        // Also can treat this as "Peer found" immediately
                                        let _ = event_tx.send(AppEvent::PeerFound {
                                            ip: addr.ip().to_string(),
                                            hostname: remote_name,
                                        });
                                    }
                                }
                                DiscoveryMsg::DiscoveryResponse {
                                    my_name: remote_name,
                                    ..
                                } => {
                                    // Found a peer!
                                    if remote_name != my_name {
                                        let _ = event_tx.send(AppEvent::PeerFound {
                                            ip: addr.ip().to_string(),
                                            hostname: remote_name,
                                        });
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
