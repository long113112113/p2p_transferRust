//! HTTP server for file sharing
//!
//! Simple HTTP server for LAN file sharing with random session token and WebSocket upload.

use crate::AppEvent;
use crate::config;
use anyhow::Result;
use axum::{
    Router,
    extract::ws::WebSocketUpgrade,
    response::{Html, Response},
    routing::get,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use super::websocket::{self, UploadState, WebSocketState};

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

/// WebSocket upgrade handler
async fn ws_upgrade_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<WebSocketState>>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<SocketAddr>,
) -> Response {
    let ip = addr.ip().to_string();
    ws.on_upgrade(move |socket| websocket::handle_socket(socket, state, ip))
}

/// Build the axum router with a dynamic token path and WebSocket support
pub fn create_router_with_websocket(
    token: &str,
    event_tx: mpsc::Sender<AppEvent>,
    upload_state: Arc<UploadState>,
    download_dir: PathBuf,
) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // Create shared WebSocket state
    let ws_state = Arc::new(WebSocketState {
        event_tx,
        upload_state,
        download_dir,
    });

    // Routes
    let index_path = format!("/{}", token);
    let ws_path = format!("/{}/ws", token);

    Router::new()
        .route(&index_path, get(index_handler))
        .route(&ws_path, get(ws_upgrade_handler))
        .fallback(not_found_handler)
        .layer(cors)
        .with_state(ws_state)
}

/// Start the HTTP server with WebSocket support
pub async fn start_http_server_with_websocket(
    addr: SocketAddr,
    token: &str,
    event_tx: mpsc::Sender<AppEvent>,
    upload_state: Arc<UploadState>,
    cancel_token: Option<CancellationToken>,
) -> Result<()> {
    let download_dir = config::get_download_dir();
    let router = create_router_with_websocket(token, event_tx, upload_state, download_dir);
    let listener = TcpListener::bind(addr).await?;

    tracing::info!("HTTP server starting on http://{}/{}", addr, token);

    if let Some(ct) = cancel_token {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            ct.cancelled().await;
            tracing::info!("HTTP server shutting down gracefully");
        })
        .await?;
    } else {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
    }

    Ok(())
}

/// Start the HTTP server on the default port with WebSocket support
pub async fn start_default_http_server_with_websocket(
    token: &str,
    event_tx: mpsc::Sender<AppEvent>,
    upload_state: Arc<UploadState>,
    cancel_token: Option<CancellationToken>,
) -> Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{}", HTTP_PORT).parse()?;
    start_http_server_with_websocket(addr, token, event_tx, upload_state, cancel_token).await
}

// Keep old function for backward compatibility (deprecated)
/// Build the axum router with a dynamic token path (no WebSocket)
#[deprecated(note = "Use create_router_with_websocket instead")]
pub fn create_router_with_token(token: &str) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let path = format!("/{}", token);
    Router::new()
        .route(&path, get(index_handler))
        .fallback(not_found_handler)
        .layer(cors)
}

/// Start the HTTP server with a session token (deprecated - no WebSocket)
#[deprecated(note = "Use start_http_server_with_websocket instead")]
pub async fn start_http_server_with_token(
    addr: SocketAddr,
    token: &str,
    cancel_token: Option<CancellationToken>,
) -> Result<()> {
    #[allow(deprecated)]
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

/// Start the HTTP server on the default port (deprecated - no WebSocket)
#[deprecated(note = "Use start_default_http_server_with_websocket instead")]
pub async fn start_default_http_server_with_token(
    token: &str,
    cancel_token: Option<CancellationToken>,
) -> Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{}", HTTP_PORT).parse()?;
    #[allow(deprecated)]
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
}
