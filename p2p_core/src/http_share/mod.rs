//! HTTP file sharing module
//!
//! Browser sharing interface with WebSocket upload support.

pub mod server;
pub mod websocket;

pub use server::{
    HTTP_PORT, generate_session_token, start_default_http_server_with_websocket,
    start_http_server_with_websocket,
};
pub use websocket::{UploadState, respond_to_upload};
