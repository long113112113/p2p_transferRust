#[cfg(test)]
mod tests {
    use p2p_core::transfer::protocol::recv_msg;
    use p2p_core::transfer::quic::{make_client_endpoint, make_server_endpoint};

    #[tokio::test]
    async fn test_max_msg_size_dos() {
        // 1. Setup Server
        let server_addr = "127.0.0.1:0".parse().unwrap();
        let server = make_server_endpoint(server_addr).unwrap();
        let server_port = server.local_addr().unwrap().port();

        // 2. Setup Client
        let client = make_client_endpoint().unwrap();
        let server_endpoint_addr = format!("127.0.0.1:{}", server_port).parse().unwrap();

        // Spawn server loop
        let server_handle = tokio::spawn(async move {
            if let Some(conn) = server.accept().await {
                let connection = conn.await.unwrap();
                if let Ok((_send, mut recv)) = connection.accept_bi().await {
                    // Try to receive a message
                    return recv_msg(&mut recv).await;
                }
            }
            Ok(p2p_core::transfer::protocol::TransferMsg::ReadyForData)
        });

        let connection = client
            .connect(server_endpoint_addr, "localhost")
            .unwrap()
            .await
            .expect("Client failed to connect");

        // 4. Client sends malicious payload
        let (mut send, _recv) = connection.open_bi().await.unwrap();

        // Send a length header that is too large (64KB + 1)
        let max_size = 64 * 1024;
        let malicious_len: u32 = (max_size + 1) as u32;

        // Use write_all but ensure AsyncWriteExt is imported implicitly or explicitly
        use tokio::io::AsyncWriteExt;

        send.write_all(&malicious_len.to_be_bytes())
            .await
            .unwrap();

        send.finish().unwrap();

        // 5. Assert result
        let result = server_handle.await.unwrap();

        match result {
            Err(e) => {
                let err_msg = e.to_string();
                assert!(err_msg.contains("Message too large"), "Expected 'Message too large' error, got: {}", err_msg);
            }
            Ok(_) => {
                panic!("Expected error, but got Ok");
            }
        }
    }
}
