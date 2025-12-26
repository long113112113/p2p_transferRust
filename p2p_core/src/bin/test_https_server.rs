//! Standalone test binary for HTTPS file sharing server
//!
//! Run with: cargo run --bin test_https_server
//!
//! This will start an HTTPS server on https://localhost:8443
//! Note: Since we use a self-signed certificate, your browser will show a security warning.
//! You can safely proceed to access the page for testing purposes.

use p2p_core::http_share::{HTTPS_PORT, start_https_server};
use std::net::SocketAddr;

#[tokio::main]
async fn main() {
    // Install the ring crypto provider for rustls
    // This must be done before any TLS operations
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let addr: SocketAddr = format!("0.0.0.0:{}", HTTPS_PORT).parse().unwrap();

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║           HTTPS File Sharing Server - Test Mode            ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!(
        "║ Server starting on: https://localhost:{}                 ║",
        HTTPS_PORT
    );
    println!("║                                                            ║");
    println!("║ ⚠️  Using self-signed certificate                          ║");
    println!("║    Your browser will show a security warning.              ║");
    println!("║    Click 'Advanced' -> 'Proceed to localhost' to access.   ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();

    if let Err(e) = start_https_server(addr).await {
        eprintln!("Server error: {}", e);
        std::process::exit(1);
    }
}
