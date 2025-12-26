//! HTTPS server for file sharing with self-signed TLS certificate
//!
//! Uses the same certificate generation logic as the QUIC transfer module.

use crate::transfer::utils::generate_self_signed_cert;
use anyhow::Result;
use axum::{Router, response::Html, routing::get};
use axum_server::tls_rustls::RustlsConfig;
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};

/// Default HTTPS port for file sharing
pub const HTTPS_PORT: u16 = 8443;

/// Static HTML content for the web interface
const INDEX_HTML: &str = include_str!("static/index.html");

/// Handler for the root route - serves the main web interface
async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Build the axum router with all routes
pub fn create_router() -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new().route("/", get(index_handler)).layer(cors)
}

/// Start the HTTPS server with self-signed TLS certificate
///
/// # Arguments
/// * `addr` - Socket address to bind to (e.g., "0.0.0.0:8443")
///
/// # Returns
/// * `Result<()>` - Ok if server started successfully
pub async fn start_https_server(addr: SocketAddr) -> Result<()> {
    // Generate self-signed certificate using shared utility
    let (certs, key) = generate_self_signed_cert()?;

    // Convert to formats expected by axum-server
    let cert_der: Vec<Vec<u8>> = certs.iter().map(|c| c.to_vec()).collect();
    let key_der = key.secret_pkcs8_der().to_vec();

    // Create RustlsConfig from DER-encoded cert and key
    let tls_config = RustlsConfig::from_der(cert_der, key_der).await?;

    let router = create_router();

    tracing::info!("HTTPS server starting on https://{}", addr);

    axum_server::bind_rustls(addr, tls_config)
        .serve(router.into_make_service())
        .await?;

    Ok(())
}

/// Start the HTTPS server on the default port
pub async fn start_default_https_server() -> Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{}", HTTPS_PORT).parse()?;
    start_https_server(addr).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_router() {
        let _router = create_router();
        // Router creation should not panic
    }
}
