use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use std::time::Duration;
use tracing::info;

use crate::protocol::ALPN;

/// Manages outbound P2P connections using Iroh
pub struct Connector {
    endpoint: Endpoint,
}

impl Connector {
    /// Creates a new connector with the given secret key
    ///
    /// # Arguments
    /// * `secret_key` - The secret key that determines the node's identity
    pub async fn new(secret_key: SecretKey) -> Result<Self> {
        info!("Initializing Iroh connector endpoint...");

        let mut transport_config = iroh::endpoint::TransportConfig::default();
        transport_config.receive_window(iroh::endpoint::VarInt::from_u32(16 * 1024 * 1024));
        transport_config.send_window(16 * 1024 * 1024);
        transport_config.initial_mtu(1200);
        transport_config.max_idle_timeout(Some(Duration::from_secs(60).try_into().unwrap()));
        transport_config.max_concurrent_bidi_streams(iroh::endpoint::VarInt::from_u32(100));
        transport_config.max_concurrent_uni_streams(iroh::endpoint::VarInt::from_u32(100));

        info!("Transport config: receive_window=16MB, send_window=16MB, mtu=1200");

        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![ALPN.to_vec()])
            .transport_config(transport_config)
            .bind()
            .await
            .context("Failed to bind connector endpoint")?;

        let node_id = endpoint.id();
        info!("Connector endpoint initialized with Node ID: {}", node_id);

        Ok(Self { endpoint })
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

    /// Connect to a remote peer by their EndpointId
    pub async fn connect(&self, target_id: EndpointId) -> Result<Connection> {
        info!("=== WAN Connection Start ===");
        info!("Target Node ID: {}", target_id);
        info!("My Node ID: {}", self.endpoint.id());
        info!("Using ALPN: {:?}", String::from_utf8_lossy(ALPN));
        info!("Attempting connection (UDP hole punch / DERP relay)...");

        let start = std::time::Instant::now();

        let connection = self
            .endpoint
            .connect(target_id, ALPN)
            .await
            .context("Failed to connect to peer")?;

        let elapsed = start.elapsed();
        info!("✓ Connected to {} in {:?}", target_id, elapsed);
        info!("Remote Node ID: {}", connection.remote_id());
        info!("=== WAN Connection Success ===");

        Ok(connection)
    }

    /// Gracefully closes the endpoint
    pub async fn close(self) -> Result<()> {
        info!("Closing connector endpoint...");
        self.endpoint.close().await;
        Ok(())
    }
}
