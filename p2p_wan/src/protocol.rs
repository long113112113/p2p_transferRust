use anyhow::Result;
use p2p_core::FileInfo;
use serde::{Deserialize, Serialize};

/// ALPN protocol identifier for doanltm-p2p
pub const ALPN: &[u8] = b"doanltm-p2p";

/// Protocol messages for WAN file transfer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WanTransferMsg {
    /// File metadata sent before transfer
    FileMetadata { info: FileInfo },
    /// Resume info with offset (0 = start from beginning)
    ResumeInfo { offset: u64 },
    /// Transfer completed successfully
    TransferComplete,
    /// Error occurred during transfer
    Error { message: String },
}

/// Send a protocol message over an iroh bidirectional stream
pub async fn send_msg(send: &mut iroh::endpoint::SendStream, msg: &WanTransferMsg) -> Result<()> {
    let json = serde_json::to_vec(msg)?;
    let len = (json.len() as u32).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(&json).await?;
    Ok(())
}

/// Receive a protocol message from an iroh bidirectional stream
pub async fn recv_msg(recv: &mut iroh::endpoint::RecvStream) -> Result<WanTransferMsg> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;

    let msg: WanTransferMsg = serde_json::from_slice(&buf)?;
    Ok(msg)
}
