//! HTTP server for file sharing
//!
//! LAN HTTP server with session tokens and WebSocket uploads.

use crate::AppEvent;
use crate::config;
use anyhow::Result;
use axum::{
    Router,
    extract::{Request, ws::WebSocketUpgrade},
    middleware::{self, Next},
    http::{HeaderValue, header},
    response::{Html, Response},
    routing::get,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
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

/// Middleware to add security headers
async fn add_security_headers(req: Request, next: Next) -> Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'self'; style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; font-src https://cdn.jsdelivr.net; script-src 'self' 'unsafe-inline'; connect-src 'self' ws: wss:; img-src 'self' data:;"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );

    response
}

/// Generate a random session token (32 characters)
pub fn generate_session_token() -> String {
    // Use full UUID entropy (128 bits) instead of 8 chars (32 bits)
    // to prevent brute-force attacks on session tokens.
    Uuid::new_v4().simple().to_string()
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
        .layer(middleware::from_fn(add_security_headers))
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
    let path = format!("/{}", token);
    Router::new()
        .route(&path, get(index_handler))
        .fallback(not_found_handler)
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
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[test]
    fn test_generate_session_token() {
        let token = generate_session_token();
        assert_eq!(token.len(), 32);
    }

    #[tokio::test]
    async fn test_cors_headers_check() {
        // Setup
        let token = "test_token";
        let (tx, _rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::default());
        let download_dir = PathBuf::from(".");
        let router = create_router_with_websocket(token, tx, upload_state, download_dir);

        // Request with Origin: http://evil.com
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", token))
                    .header("Origin", "http://evil.com")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Verify CORS headers are ABSENT (After Fix)
        // This ensures the server does not explicitly allow cross-origin requests,
        // so the browser will block them by default.
        assert!(response.headers().get("access-control-allow-origin").is_none());
    }

    #[tokio::test]
    async fn test_security_headers() {
        let token = "test_token";
        let (tx, _rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::default());
        let download_dir = PathBuf::from(".");
        let router = create_router_with_websocket(token, tx, upload_state, download_dir);

        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let headers = response.headers();
        assert!(headers.get("content-security-policy").is_some());
        assert_eq!(headers.get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(headers.get("x-frame-options").unwrap(), "DENY");
        assert_eq!(headers.get("referrer-policy").unwrap(), "no-referrer");
    }
}
