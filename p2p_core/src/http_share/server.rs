//! HTTP server for file sharing
//!
//! LAN HTTP server with session tokens and WebSocket uploads.

use crate::AppEvent;
use crate::config;
use anyhow::Result;
use axum::{
    Router,
    extract::{Request, ws::WebSocketUpgrade},
    http::{HeaderValue, header},
    middleware::{self, Next},
    response::{Html, Response},
    routing::get,
};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicUsize};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::websocket::{self, UploadState, WebSocketState};

/// Default HTTP port for file sharing
pub const HTTP_PORT: u16 = 8080;

/// Static HTML content for the web interface
const INDEX_HTML: &str = include_str!("static/index.html");

/// Static JS content for the web interface
const APP_JS: &str = include_str!("static/app.js");

/// Static CSS content for the web interface
const STYLE_CSS: &str = include_str!("static/style.css");

/// Static HTML content for the 404 page
const NOT_FOUND_HTML: &str = include_str!("static/404.html");

/// Handler for the share route - serves the main web interface
async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}

/// Handler for app.js
async fn js_handler() -> impl axum::response::IntoResponse {
    ([(header::CONTENT_TYPE, "application/javascript")], APP_JS)
}

/// Handler for style.css
async fn css_handler() -> impl axum::response::IntoResponse {
    ([(header::CONTENT_TYPE, "text/css")], STYLE_CSS)
}

/// Handler for invalid routes - serves 404 page
async fn not_found_handler() -> (axum::http::StatusCode, Html<&'static str>) {
    (axum::http::StatusCode::NOT_FOUND, Html(NOT_FOUND_HTML))
}

/// Sanitize Host header to prevent injection
fn sanitize_host(host: &str) -> String {
    let allowed = |c: char| c.is_alphanumeric() || matches!(c, '.' | ':' | '-' | '[' | ']');
    if host.chars().all(allowed) {
        host.to_string()
    } else {
        "localhost".to_string()
    }
}

/// Middleware to add security headers
async fn add_security_headers(req: Request, next: Next) -> Response {
    // Extract and sanitize Host header for dynamic CSP
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(sanitize_host)
        .unwrap_or_else(|| "self".to_string());

    // If host is "self" (fallback), we can't construct valid ws:// URL easily without knowing scheme/port,
    // so we just fallback to 'self' which might break WS if scheme is different (http vs ws).
    // But browsers usually send Host.
    let connect_src = if host == "self" {
        "connect-src 'self';".to_string()
    } else {
        format!("connect-src 'self' ws://{} wss://{};", host, host)
    };

    let csp = format!(
        "default-src 'self'; style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; font-src https://cdn.jsdelivr.net; script-src 'self'; {} img-src 'self' data:;",
        connect_src
    );

    let mut response = next.run(req).await;
    let headers = response.headers_mut();

    if let Ok(csp_value) = HeaderValue::from_str(&csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp_value);
    } else {
        // Fallback if something goes wrong with formatting (unlikely due to sanitization)
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src 'self'; style-src 'self' 'unsafe-inline' https://cdn.jsdelivr.net; font-src https://cdn.jsdelivr.net; script-src 'self'; connect-src 'self'; img-src 'self' data:;"),
        );
    }

    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
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
        connection_count: AtomicUsize::new(0),
    });

    // Routes
    let index_path = format!("/{}", token);
    let ws_path = format!("/{}/ws", token);

    Router::new()
        .route(&index_path, get(index_handler))
        .route(&ws_path, get(ws_upgrade_handler))
        .route("/app.js", get(js_handler))
        .route("/style.css", get(css_handler))
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
        assert!(
            response
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );

        // Verify CSP header
        let csp = response
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("script-src 'self'"));
        assert!(!csp.contains("script-src 'self' 'unsafe-inline'"));
    }

    #[tokio::test]
    async fn test_max_pending_uploads_limit() {
        use crate::http_share::websocket::{ClientMessage, MAX_PENDING_UPLOADS};
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;

        // Setup
        let token = "test_token_limit";
        let (tx, _rx) = mpsc::channel(100);
        // Keep rx alive to prevent channel closed errors (though ignored in handler)
        // But more importantly, if we don't hold it, it might affect test behavior if we needed to check events
        let upload_state = Arc::new(UploadState::default());
        let download_dir = PathBuf::from("."); // Mock path

        // Create router manually to get the port
        let router = create_router_with_websocket(token, tx, upload_state.clone(), download_dir);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Spawn server
        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        // Give server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let ws_url = format!("ws://127.0.0.1:{}/{}/ws", port, token);
        let mut clients = Vec::new();

        // Connect MAX + 2 clients
        for i in 0..(MAX_PENDING_UPLOADS + 2) {
            let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
            let (mut write, read) = ws_stream.split();

            // Send FileInfo to trigger pending state
            let msg = ClientMessage::FileInfo {
                file_name: format!("file_{}.txt", i),
                file_size: 100,
            };
            write
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::to_string(&msg).unwrap().into(),
                ))
                .await
                .unwrap();

            // Keep write alive to prevent connection closing
            clients.push((write, read));
        }

        // Allow some time for processing
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        // Check results
        let mut rejected_count = 0;

        for (_write, read) in clients.iter_mut() {
            // Check for rejection message
            // We use a timeout because accepted clients might not receive anything until user action
            if let Ok(Some(Ok(msg))) =
                tokio::time::timeout(tokio::time::Duration::from_millis(100), read.next()).await
            {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                    if text.contains("Too many pending uploads") {
                        rejected_count += 1;
                    }
                }
            }
        }

        let pending_count = upload_state.pending.read().await.len();

        // We expect at least 1 rejection (actually exactly 2 if we sent MAX+2 and all processed)
        assert!(rejected_count > 0, "Should have rejected some requests");

        // Check server state
        // Should be exactly MAX_PENDING_UPLOADS (10)
        assert_eq!(pending_count, MAX_PENDING_UPLOADS);
    }

    #[tokio::test]
    async fn test_request_id_length() {
        use crate::AppEvent;
        use crate::http_share::websocket::ClientMessage;
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;

        // Setup
        let token = "test_token_uuid";
        let (tx, mut rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::default());
        let download_dir = PathBuf::from("."); // Mock path

        // Create router manually to get the port
        let router =
            create_router_with_websocket(token, tx.clone(), upload_state.clone(), download_dir);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Spawn server
        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        // Give server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let ws_url = format!("ws://127.0.0.1:{}/{}/ws", port, token);

        // Connect client
        let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
        let (mut write, _read) = ws_stream.split();

        // Send FileInfo
        let msg = ClientMessage::FileInfo {
            file_name: "test.txt".to_string(),
            file_size: 100,
        };
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::to_string(&msg).unwrap().into(),
            ))
            .await
            .unwrap();

        // Wait for AppEvent::UploadRequest
        // We might get other events (like Status), so loop until we find it or timeout
        let event = tokio::time::timeout(tokio::time::Duration::from_secs(2), async {
            while let Some(evt) = rx.recv().await {
                if let AppEvent::UploadRequest { request_id, .. } = evt {
                    return Some(request_id);
                }
            }
            None
        })
        .await
        .expect("Timeout waiting for UploadRequest");

        if let Some(request_id) = event {
            assert_eq!(
                request_id.len(),
                32,
                "Request ID should be 32 characters long (UUID simple hex)"
            );
        } else {
            panic!("Expected UploadRequest event");
        }
    }

    #[tokio::test]
    async fn test_websocket_handshake_strictness() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message;

        // Setup
        let token = "test_token_strict";
        let (tx, _rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::default());
        let download_dir = PathBuf::from(".");

        // Create router manually to get the port
        let router = create_router_with_websocket(token, tx, upload_state, download_dir);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Spawn server
        tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });

        // Give server a moment to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let ws_url = format!("ws://127.0.0.1:{}/{}/ws", port, token);

        // Connect client
        let (ws_stream, _) = connect_async(&ws_url).await.expect("Failed to connect");
        let (mut write, mut read) = ws_stream.split();

        // Send INVALID message (not FileInfo)
        write
            .send(Message::Text("this is not json".into()))
            .await
            .unwrap();

        // Expect immediate closure or error
        // The server currently (before fix) ignores it and loops until timeout (10s)
        // After fix, it should return None/Error immediately.

        let result =
            tokio::time::timeout(tokio::time::Duration::from_millis(2000), read.next()).await;

        match result {
            Ok(Some(Ok(msg))) => {
                // We expect an error message or close frame
                if let Message::Close(_) = msg {
                    // Good
                } else if let Message::Text(text) = msg {
                    // "Expected file_info message" is sent if wait_for_file_info returns None
                    // "Handshake timed out" is sent if timeout
                    // We expect "Expected file_info message" or immediate closure.
                    // If we got "Handshake timed out", it means it waited too long (but my timeout is 2s, handshake is 10s).
                    // So if we get a message within 2s, it's likely "Expected file_info message".
                    assert!(
                        text.contains("Expected file_info message"),
                        "Unexpected message: {}",
                        text
                    );
                } else {
                    panic!("Unexpected message type: {:?}", msg);
                }
            }
            Ok(Some(Err(e))) => panic!("WebSocket error: {}", e),
            Ok(None) => {
                // Stream closed
            }
            Err(_) => {
                panic!("Timeout! Server did not close connection on invalid input.");
            }
        }
    }

    #[tokio::test]
    async fn test_csp_connect_src_strictness() {
        // Setup
        let token = "test_token_csp";
        let (tx, _rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::default());
        let download_dir = PathBuf::from(".");
        let router = create_router_with_websocket(token, tx, upload_state, download_dir);

        let host = "example.com";

        // Request with Host: example.com
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", token))
                    .header("Host", host)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Verify CSP header
        let csp = response
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();

        // Should contain specific host
        assert!(
            csp.contains(&format!("ws://{}", host)),
            "CSP should allow ws://{{host}} but got: {}", csp
        );
        assert!(
            csp.contains(&format!("wss://{}", host)),
            "CSP should allow wss://{{host}} but got: {}", csp
        );

        // Should NOT contain wildcards
        assert!(
            !csp.contains(" ws: "),
            "CSP should not contain wildcard 'ws:' but got: {}", csp
        );
        assert!(
            !csp.contains(" wss: "),
            "CSP should not contain wildcard 'wss:' but got: {}", csp
        );
    }

    #[tokio::test]
    async fn test_csp_injection_prevention() {
        // Setup
        let token = "test_token_csp_inject";
        let (tx, _rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::default());
        let download_dir = PathBuf::from(".");
        let router = create_router_with_websocket(token, tx, upload_state, download_dir);

        // Malicious Host header attempting to inject strict-dynamic or unsafe-inline
        let malicious_host = "evil.com; script-src 'unsafe-inline'";

        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", token))
                    .header("Host", malicious_host)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let csp = response
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();

        // Should NOT contain the injected host
        assert!(
            !csp.contains("evil.com"),
            "CSP should not contain malicious host part but got: {}", csp
        );

        // Should fall back to localhost
        assert!(
            csp.contains("ws://localhost"),
            "CSP should fall back to localhost for invalid host but got: {}", csp
        );
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_csp_connect_src_strictness() {
        let token = "test_token_csp";
        let (tx, _rx) = mpsc::channel(100);
        let upload_state = Arc::new(UploadState::default());
        let download_dir = PathBuf::from(".");
        let router = create_router_with_websocket(token, tx, upload_state, download_dir);

        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/{}", token))
                    .header("Host", "127.0.0.1:8080")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let csp = response
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();

        // Assert STRICT state
        // Should contain specific host
        assert!(csp.contains("ws://127.0.0.1:8080"));
        assert!(csp.contains("wss://127.0.0.1:8080"));

        // Should NOT contain the old permissive wildcards
        assert!(!csp.contains("ws: wss:"));
    }

    #[test]
    fn test_sanitize_host() {
        assert_eq!(sanitize_host("example.com"), "example.com");
        assert_eq!(sanitize_host("localhost:8080"), "localhost:8080");
        assert_eq!(sanitize_host("[::1]:8080"), "[::1]:8080");

        // Injection attempts
        assert_eq!(sanitize_host("example.com; script-src 'unsafe-inline'"), "localhost");
        assert_eq!(sanitize_host("evil.com\r\nHeader: value"), "localhost");
    }
}
