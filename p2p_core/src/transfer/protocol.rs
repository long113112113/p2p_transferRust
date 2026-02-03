use crate::FileInfo;
use super::constants::MAX_MESSAGE_SIZE;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt};

/// Protocol messages for transfer handshake
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransferMsg {
    PairingRequest {
        endpoint_id: String,
        peer_name: String,
    },
    PairingAccepted,
    VerificationRequired,
    VerificationCode {
        code: String,
    },
    VerificationSuccess,
    VerificationFailed {
        message: String,
    },
    FileMetadata {
        info: FileInfo,
    },
    ReadyForData,
    ResumeInfo {
        offset: u64,
    },
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
pub async fn recv_msg<R: AsyncRead + Unpin>(recv: &mut R) -> Result<TransferMsg> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    if len > MAX_MESSAGE_SIZE {
        return Err(anyhow::anyhow!(
            "Message too large: {} bytes (max {})",
            len,
            MAX_MESSAGE_SIZE
        ));
    }

    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf).await?;

    let msg: TransferMsg = serde_json::from_slice(&buf)?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncWriteExt, duplex};

    #[tokio::test]
    async fn test_recv_msg_limit() {
        // Create a duplex stream (mocking network)
        let (mut client, mut server) = duplex(1024 * 1024); // 1MB buffer

        // Simulate a malicious message
        // Length = MAX_MESSAGE_SIZE + 1
        let malicious_len = (MAX_MESSAGE_SIZE + 1) as u32;

        // Write length
        client.write_all(&malicious_len.to_be_bytes()).await.unwrap();
        // We don't even need to write the body, because it should fail at length check

        // Try to receive
        let result = recv_msg(&mut server).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("Message too large"));
    }

    #[tokio::test]
    async fn test_recv_msg_valid() {
        let (mut client, mut server) = duplex(1024 * 1024);

        let msg = TransferMsg::TransferComplete;

        // Send valid message
        tokio::spawn(async move {
            // We can't use send_msg because it requires quinn::SendStream,
            // but we can manually write what send_msg does.
            let json = serde_json::to_vec(&msg).unwrap();
            let len = (json.len() as u32).to_be_bytes();
            client.write_all(&len).await.unwrap();
            client.write_all(&json).await.unwrap();
        });

        let result = recv_msg(&mut server).await;
        assert!(result.is_ok());
        match result.unwrap() {
            TransferMsg::TransferComplete => {},
            _ => panic!("Wrong message type"),
        }
    }
}
