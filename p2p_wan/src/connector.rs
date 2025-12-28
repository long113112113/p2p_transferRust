use anyhow::{Context, Result};
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
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

        let endpoint = Endpoint::builder()
            .secret_key(secret_key)
            .alpns(vec![ALPN.to_vec()])
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
    ///
    /// Iroh will automatically:
    /// - Try UDP hole punching for direct connection
    /// - Fall back to DERP relay if direct connection fails
    pub async fn connect(&self, target_id: EndpointId) -> Result<Connection> {
        info!("Connecting to peer: {}", target_id);

        let connection = self
            .endpoint
            .connect(target_id, ALPN)
            .await
            .context("Failed to connect to peer")?;

        info!("Successfully connected to peer: {}", target_id);
        Ok(connection)
    }

    /// Gracefully closes the endpoint
    pub async fn close(self) -> Result<()> {
        info!("Closing connector endpoint...");
        self.endpoint.close().await;
        Ok(())
    }
}
