//! HTTP server for file sharing
//!
//! Simple HTTP server for LAN file sharing with random session token.

use anyhow::Result;
use axum::{Router, response::Html, routing::get};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

/// Default HTTP port for file sharing
pub const HTTP_PORT: u16 = 8080;

/// Static HTML content for the web interface
const INDEX_HTML: &str = include_str!("static/index.html");

/// Static HTML content for the 404 page
const NOT_FOUND_HTML: &str = include_str!("static/404.html");

/// Handler for the share route - serves the main web interface
async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Handler for invalid routes - serves 404 page
async fn not_found_handler() -> (axum::http::StatusCode, Html<&'static str>) {
    (axum::http::StatusCode::NOT_FOUND, Html(NOT_FOUND_HTML))
}

/// Generate a random session token (8 characters)
pub fn generate_session_token() -> String {
    Uuid::new_v4().to_string()[..8].to_string()
}

/// Build the axum router with a dynamic token path
pub fn create_router_with_token(token: &str) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Route: /{token} for valid access, fallback for everything else
    let path = format!("/{}", token);
    Router::new()
        .route(&path, get(index_handler))
        .fallback(not_found_handler)
        .layer(cors)
}

/// Start the HTTP server with a session token
///
/// # Arguments
/// * `addr` - Socket address to bind to (e.g., "0.0.0.0:8080")
/// * `token` - Session token for the URL path
/// * `cancel_token` - Optional cancellation token for graceful shutdown
///
/// # Returns
/// * `Result<()>` - Ok if server started successfully
pub async fn start_http_server_with_token(
    addr: SocketAddr,
    token: &str,
    cancel_token: Option<CancellationToken>,
) -> Result<()> {
    let router = create_router_with_token(token);
    let listener = TcpListener::bind(addr).await?;

    tracing::info!("HTTP server starting on http://{}/{}", addr, token);

    if let Some(ct) = cancel_token {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                ct.cancelled().await;
                tracing::info!("HTTP server shutting down gracefully");
            })
            .await?;
    } else {
        axum::serve(listener, router).await?;
    }

    Ok(())
}

/// Start the HTTP server on the default port with a token and optional cancellation
pub async fn start_default_http_server_with_token(
    token: &str,
    cancel_token: Option<CancellationToken>,
) -> Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{}", HTTP_PORT).parse()?;
    start_http_server_with_token(addr, token, cancel_token).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_session_token() {
        let token = generate_session_token();
        assert_eq!(token.len(), 8);
    }

    #[test]
    fn test_create_router_with_token() {
        let _router = create_router_with_token("abc123");
        // Router creation should not panic
    }
}
