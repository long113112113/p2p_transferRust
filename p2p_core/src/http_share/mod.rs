//! HTTP/HTTPS file sharing module
//!
//! Provides a web interface for sharing files via browser.

pub mod server;

pub use server::{HTTPS_PORT, create_router, start_default_https_server, start_https_server};
