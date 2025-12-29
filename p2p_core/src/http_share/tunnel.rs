//! Bore tunnel for WAN access
//!
//! Provides public URL tunneling via bore.pub relay server.

use anyhow::Result;
use bore_cli::client::Client;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

/// Default bore relay server
pub const BORE_SERVER: &str = "bore.pub";

/// Bore tunnel state
pub struct BoreTunnel {
    remote_port: u16,
    cancel_token: CancellationToken,
}

/// Shared tunnel state for async access
pub type SharedTunnel = Arc<RwLock<Option<BoreTunnel>>>;

impl BoreTunnel {
    /// Start a new bore tunnel to the local HTTP server
    ///
    /// Returns the tunnel instance with the assigned remote port.
    /// The tunnel runs in a background task until cancelled.
    pub async fn start(local_port: u16) -> Result<Self> {
        // Create bore client connecting to bore.pub
        let client = Client::new(
            "localhost",
            local_port,
            BORE_SERVER,
            0, // 0 = random port
            None,
        )
        .await?;

        let remote_port = client.remote_port();
        let cancel_token = CancellationToken::new();
        let cancel_clone = cancel_token.clone();

        // Spawn tunnel listener in background
        tokio::spawn(async move {
            tokio::select! {
                result = client.listen() => {
                    if let Err(e) = result {
                        tracing::error!("Bore tunnel error: {}", e);
                    }
                }
                _ = cancel_clone.cancelled() => {
                    tracing::info!("Bore tunnel cancelled");
                }
            }
        });

        tracing::info!("Bore tunnel started: {}:{}", BORE_SERVER, remote_port);

        Ok(Self {
            remote_port,
            cancel_token,
        })
    }

    /// Get the public URL for the tunnel
    pub fn public_url(&self, session_token: &str) -> String {
        format!(
            "http://{}:{}/{}",
            BORE_SERVER, self.remote_port, session_token
        )
    }

    /// Get the remote port
    pub fn remote_port(&self) -> u16 {
        self.remote_port
    }

    /// Stop the tunnel
    pub fn stop(&self) {
        self.cancel_token.cancel();
        tracing::info!("Bore tunnel stopped");
    }
}

impl Drop for BoreTunnel {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}
