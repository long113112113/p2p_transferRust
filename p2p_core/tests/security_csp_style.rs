use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use p2p_core::http_share::server::create_router_with_websocket;
use p2p_core::http_share::websocket::UploadState;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tower::ServiceExt;

#[tokio::test]
async fn test_csp_style_src_no_unsafe_inline() {
    // Setup
    let token = "test_csp_token";
    let (tx, _rx) = mpsc::channel(100);
    let upload_state = Arc::new(UploadState::default());
    let download_dir = PathBuf::from(".");
    let router = create_router_with_websocket(token, tx, upload_state, download_dir);

    // Make request
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/{}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Verify CSP
    let csp = response
        .headers()
        .get("content-security-policy")
        .expect("CSP header missing")
        .to_str()
        .unwrap();

    // Check that style-src exists
    assert!(csp.contains("style-src"));

    assert!(
        !csp.contains("'unsafe-inline'"),
        "CSP should not contain 'unsafe-inline' but got: {}",
        csp
    );

    // Check allow list
    assert!(
        csp.contains("style-src 'self' https://cdn.jsdelivr.net"),
        "CSP should allow self and jsdelivr for styles, but got: {}",
        csp
    );
}
