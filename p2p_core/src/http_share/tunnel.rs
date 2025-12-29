//! Ngrok tunnel for WAN access
//!
//! Provides public HTTPS URL tunneling via ngrok service.
//! Requires NGROK_AUTHTOKEN environment variable or config file.

use anyhow::Result;
use ngrok::config::ForwarderBuilder;
use ngrok::forwarder::Forwarder;
use ngrok::tunnel::{EndpointInfo, HttpTunnel};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use url::Url;

/// Shared tunnel state for async access
pub type SharedTunnel = Arc<RwLock<Option<NgrokTunnel>>>;

/// Ngrok tunnel state
pub struct NgrokTunnel {
    public_url: String,
    cancel_token: CancellationToken,
}

impl NgrokTunnel {
    /// Start a new ngrok tunnel forwarding to the local HTTP server
    ///
    /// Returns the tunnel instance with the public HTTPS URL.
    /// The tunnel runs in a background task until cancelled.
    ///
    /// Requires NGROK_AUTHTOKEN environment variable to be set.
    pub async fn start(local_port: u16, session_token: &str) -> Result<Self> {
        // Build ngrok session (will read NGROK_AUTHTOKEN from env)
        let session = ngrok::Session::builder()
            .authtoken_from_env()
            .connect()
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to connect to ngrok: {}. Make sure NGROK_AUTHTOKEN is set.",
                    e
                )
            })?;

        // Create HTTP tunnel forwarding to local server
        // Forward to the root port, as the server handles the /token logic
        let local_url = format!("http://localhost:{}", local_port);
        let tunnel_builder = session.http_endpoint();

        let forwarder: Forwarder<HttpTunnel> = tunnel_builder
            .listen_and_forward(Url::parse(&local_url)?)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to start ngrok tunnel: {}", e))?;

        let public_url = format!("{}/{}", forwarder.url(), session_token);
        let cancel_token = CancellationToken::new();
        let cancel_clone = cancel_token.clone();

        tracing::info!("Ngrok tunnel started: {}", public_url);

        // The forwarder runs automatically, we just need to keep it alive
        tokio::spawn(async move {
            tokio::select! {
                _ = cancel_clone.cancelled() => {
                    tracing::info!("Ngrok tunnel cancelled");
                    // Forwarder will be dropped here, closing the tunnel
                }
                _ = async {
                    // Keep the forwarder alive
                    loop {
                        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                    }
                } => {}
            }
            drop(forwarder);
        });

        Ok(Self {
            public_url,
            cancel_token,
        })
    }

    /// Get the public HTTPS URL for the tunnel
    pub fn public_url(&self) -> &str {
        &self.public_url
    }

    /// Stop the tunnel
    pub fn stop(&self) {
        self.cancel_token.cancel();
        tracing::info!("Ngrok tunnel stopped");
    }
}

impl Drop for NgrokTunnel {
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}
