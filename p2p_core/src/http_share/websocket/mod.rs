//! WebSocket handler for file upload
//!
//! Handles binary file uploads from web clients via WebSocket.

mod handler;
mod messages;
mod state;
mod utils;

pub use handler::handle_socket;
pub use messages::{
    CHUNK_SIZE, MAX_PENDING_UPLOADS, ClientMessage, ServerMessage, USER_RESPONSE_TIMEOUT_SECS,
};
pub use state::{PendingUpload, UploadState, WebSocketState, respond_to_upload};

use axum::{
    extract::{State, WebSocketUpgrade},
    response::Response,
};
use std::{net::SocketAddr, sync::Arc};

/// WebSocket upgrade handler
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<WebSocketState>>,
    addr: Option<axum::extract::ConnectInfo<SocketAddr>>,
) -> Response {
    let client_ip = addr
        .map(|a| a.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    ws.on_upgrade(move |socket| handle_socket(socket, state, client_ip))
}
