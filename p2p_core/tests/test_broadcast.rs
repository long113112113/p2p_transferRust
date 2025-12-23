use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::timeout;

#[tokio::test]
async fn test_broadcast_self_receive() {
    // 1. Bind to any available port on all interfaces
    // We bind to 0.0.0.0 to listen for broadcast packets
    let socket = UdpSocket::bind("0.0.0.0:0").await.expect("Failed to bind");
    let local_addr = socket.local_addr().expect("Failed to get local addr");
    let port = local_addr.port();

    // 2. Enable broadcasting
    socket.set_broadcast(true).expect("Failed to set broadcast");

    // 3. Prepare message
    let test_msg = b"BROADCAST_TEST_PAYLOAD";

    // We send to 255.255.255.255 (Limited Broadcast Address)
    // to the same port we are listening on.
    let broadcast_addr = format!("255.255.255.255:{}", port);

    println!("Sending broadcast to {}...", broadcast_addr);

    // 4. Send message to broadcast address
    socket
        .send_to(test_msg, &broadcast_addr)
        .await
        .expect("Failed to send broadcast");

    // 5. Try to receive with a 2-second timeout
    // Note: receiving your own broadcast is typically supported on Linux/Windows
    // if the socket is bound to 0.0.0.0 and broadcast is enabled.
    let mut buf = [0u8; 1024];
    let result = timeout(Duration::from_secs(2), socket.recv_from(&mut buf)).await;

    match result {
        Ok(Ok((len, addr))) => {
            let received_msg = &buf[..len];
            println!("Received {} bytes from {}", len, addr);
            assert_eq!(received_msg, test_msg, "Received message content mismatch");
            println!("Success: Network supports broadcasting and loopback receiving!");
        }
        Ok(Err(e)) => panic!("Error during receive: {}", e),
        Err(_) => {
            println!("Failure: Timeout - Broadcast packet not received.");
            println!("Reasons could be:");
            println!("1. OS loopback for broadcast is disabled.");
            println!(
                "2. Firewall is blocking outgoing or incoming UDP broadcast on port {}.",
                port
            );
            println!("3. Network interface does not support broadcasting.");
            panic!("Broadcast test failed due to timeout");
        }
    }
}
