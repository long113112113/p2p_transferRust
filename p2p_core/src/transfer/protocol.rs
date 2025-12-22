use crate::FileInfo;
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Protocol messages for transfer handshake
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransferMsg {
    /// Sender initiates connection with their identity
    PairingRequest { peer_id: String, peer_name: String },
    /// Receiver responds: already paired, proceed
    PairingAccepted,
    /// Receiver responds: need verification, show code to user
    VerificationRequired,
    /// Sender submits the 4-digit code
    VerificationCode { code: String },
    /// Verification successful, can proceed with transfer
    VerificationSuccess,
    /// Verification failed
    VerificationFailed { message: String },
    /// File transfer metadata
    FileMetadata { info: FileInfo },
    /// Ready to receive file data
    ReadyForData,
    /// Resume transfer from offset
    ResumeInfo { offset: u64 },
    /// Receiver confirms file was received successfully
    TransferComplete,
}

/// Send a protocol message over a bidirectional stream
pub async fn send_msg(send: &mut quinn::SendStream, msg: &TransferMsg) -> Result<()> {
    let json = serde_json::to_vec(msg)?;
    let len = (json.len() as u32).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(&json).await?;
    Ok(())
}

/// Receive a protocol message from a bidirectional stream
pub async fn recv_msg(recv: &mut quinn::RecvStream) -> Result<TransferMsg> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;

    let msg: TransferMsg = serde_json::from_slice(&buf)?;
    Ok(msg)
}
