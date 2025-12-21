//! QUIC-based file transfer module using quinn.
//!
//! This module provides:
//! - Self-signed certificate generation
//! - QUIC server endpoint (to receive files)
//! - QUIC client endpoint (to send files)
//! - Verification handshake with 4-digit code

mod constants;
mod hash;
mod protocol;
mod quic;
mod receiver;
mod sender;
mod server;

// Re-export public API
pub use constants::*;
pub use protocol::TransferMsg;
pub use quic::{make_client_endpoint, make_server_endpoint};
pub use sender::send_files;
pub use server::run_server;
