//! HTTP file sharing module
//!
//! Provides a web interface for sharing files via browser.

pub mod server;

pub use server::{HTTP_PORT, generate_session_token, start_default_http_server_with_token};
