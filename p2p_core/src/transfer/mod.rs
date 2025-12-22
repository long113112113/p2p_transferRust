//! QUIC-based file transfer module using quinn.
//!
//! This module provides:
//! - Self-signed certificate generation
//! - QUIC server endpoint (to receive files)
//! - QUIC client endpoint (to send files)
//! - Verification handshake with 4-digit code

pub mod constants;
pub mod hash;
pub mod protocol;
pub mod quic;
pub mod receiver;
pub mod sender;
pub mod server;
pub mod utils;

// Re-export public API
pub use constants::TRANSFER_PORT;
pub use quic::{make_client_endpoint, make_server_endpoint};
pub use sender::{TransferContext, send_files};
pub use server::run_server;
