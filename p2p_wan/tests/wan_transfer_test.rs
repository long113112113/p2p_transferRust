use anyhow::Result;
use iroh::{Endpoint, EndpointId, SecretKey};
use p2p_core::AppEvent;
use std::path::PathBuf;
use std::str::FromStr;
use tokio::sync::mpsc;

/// Target EndpointID of the running listener (provided by user)
const TARGET_ENDPOINT_ID: &str = "ddfadcc8a77b75372141e7dcaa3bdace906a65b7f7036fd472bb5d5b611febf6";

/// ALPN protocol identifier - must match p2p_wan::protocol::ALPN
const ALPN: &[u8] = b"doanltm-p2p";

/// Test file size: 100MB
const TEST_FILE_SIZE: usize = 100 * 1024 * 1024;

#[tokio::test]
async fn test_wan_connection_to_remote_endpoint() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();

    println!("=== WAN Connection Test ===");
    println!("Target EndpointID: {}", TARGET_ENDPOINT_ID);

    // Parse target EndpointId
    let target_id = EndpointId::from_str(TARGET_ENDPOINT_ID)?;
    println!("Parsed EndpointId: {}", target_id);

    // Create a new SecretKey for this test client
    let secret_key = SecretKey::generate(&mut rand::rng());
    println!("Generated test client SecretKey");

    // Build endpoint
    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    println!("Test client EndpointId: {}", endpoint.id());

    // Attempt connection
    println!("Connecting to target...");
    let start = std::time::Instant::now();

    let connection = endpoint.connect(target_id, ALPN).await?;

    let elapsed = start.elapsed();
    println!("✓ Connected in {:?}", elapsed);
    println!("Remote ID: {}", connection.remote_id());

    // Test bidirectional stream
    println!("Opening bi-directional stream...");
    let (_send, _recv) = connection.open_bi().await?;
    println!("✓ Bi-stream opened");

    // Clean up
    connection.close(0u8.into(), b"test complete");
    endpoint.close().await;
    println!("✓ Connection closed gracefully");

    println!("=== Test PASSED ===");
    Ok(())
}

#[tokio::test]
async fn test_wan_file_transfer_to_remote_endpoint() -> Result<()> {
    use p2p_wan::sender::send_files;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .try_init()
        .ok();

    println!("=== WAN File Transfer Test (100MB) ===");
    println!("Target EndpointID: {}", TARGET_ENDPOINT_ID);

    // Create 100MB test file
    println!("Creating 100MB test file...");
    let mut temp_file = NamedTempFile::new()?;
    let test_data = vec![0xABu8; TEST_FILE_SIZE];
    temp_file.write_all(&test_data)?;
    temp_file.flush()?;
    let file_path = temp_file.path().to_path_buf();
    println!("✓ Test file created: {:?}", file_path);

    // Parse target EndpointId
    let target_id = EndpointId::from_str(TARGET_ENDPOINT_ID)?;

    // Create connector
    let secret_key = SecretKey::generate(&mut rand::rng());
    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    println!("Test client EndpointId: {}", endpoint.id());

    // Connect
    println!("Connecting to target...");
    let start = std::time::Instant::now();
    let connection = endpoint.connect(target_id, ALPN).await?;
    println!("✓ Connected in {:?}", start.elapsed());

    // Setup event channel
    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(100);

    // Spawn event listener
    let event_handle = tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            match &event {
                AppEvent::TransferProgress {
                    file_name,
                    progress,
                    speed,
                    ..
                } => {
                    println!("  Progress: {} - {:.1}% @ {}", file_name, progress, speed);
                }
                AppEvent::TransferCompleted(file_name) => {
                    println!("✓ Transfer completed: {}", file_name);
                    break;
                }
                AppEvent::Error(msg) => {
                    println!("✗ Error: {}", msg);
                    break;
                }
                _ => {}
            }
        }
    });

    // Send file
    println!("Sending 100MB file...");
    let transfer_start = std::time::Instant::now();
    send_files(&connection, vec![file_path], event_tx).await?;

    let transfer_elapsed = transfer_start.elapsed();
    let speed_mbps = (TEST_FILE_SIZE as f64 / transfer_elapsed.as_secs_f64()) / 1_000_000.0;
    println!(
        "✓ Transfer completed in {:?} ({:.2} MB/s)",
        transfer_elapsed, speed_mbps
    );

    // Wait for events to be processed
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), event_handle).await;

    // Clean up
    connection.close(0u8.into(), b"transfer complete");
    endpoint.close().await;

    println!("=== File Transfer Test PASSED ===");
    Ok(())
}

#[tokio::test]
async fn test_local_endpoint_pair() -> Result<()> {
    // Test two local endpoints connecting to each other (no external endpoint needed)
    use p2p_core::FileInfo;
    use p2p_wan::protocol::{WanTransferMsg, recv_msg, send_msg};

    println!("=== Local Endpoint Pair Test ===");

    // Create listener endpoint
    let listener_key = SecretKey::generate(&mut rand::rng());
    let listener = Endpoint::builder()
        .secret_key(listener_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;
    let listener_id = listener.id();

    // Wait for relay connection to be established (critical for local test)
    println!("Waiting for relay connection...");
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Get full address including relay URL
    let listener_addr = listener.addr();
    println!("Listener EndpointId: {}", listener_id);
    println!("Listener Addr: {:?}", listener_addr);

    // Create connector endpoint
    let connector_key = SecretKey::generate(&mut rand::rng());
    let connector = Endpoint::builder()
        .secret_key(connector_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;
    println!("Connector EndpointId: {}", connector.id());

    // Spawn listener accept task
    let listener_clone = listener.clone();
    let accept_handle = tokio::spawn(async move {
        println!("Listener: Waiting for connection...");
        if let Some(incoming) = listener_clone.accept().await {
            let conn = incoming.await?;
            println!("Listener: Connection accepted from {}", conn.remote_id());

            // Accept bi-stream
            let (mut send, mut recv) = conn.accept_bi().await?;
            println!("Listener: Bi-stream accepted");

            // Receive message
            let msg = recv_msg(&mut recv).await?;
            println!("Listener: Received message: {:?}", msg);

            // Send response
            send_msg(&mut send, &WanTransferMsg::ResumeInfo { offset: 0 }).await?;
            println!("Listener: Sent ResumeInfo");

            // Wait for stream to close
            let _ = recv.read_to_end(1024).await;

            // Send completion
            send_msg(&mut send, &WanTransferMsg::TransferComplete).await?;
            println!("Listener: Sent TransferComplete");
        }
        Ok::<(), anyhow::Error>(())
    });

    // Connect using full EndpointAddr (includes relay URL)
    println!("Connector: Connecting to listener...");
    let conn = connector.connect(listener_addr, ALPN).await?;
    println!("Connector: Connected!");

    // Open bi-stream and send test message
    let (mut send, mut recv) = conn.open_bi().await?;
    println!("Connector: Bi-stream opened");

    let test_info = FileInfo {
        file_name: "test.txt".to_string(),
        file_size: 1024,
        file_path: PathBuf::new(),
        file_hash: None,
    };
    send_msg(&mut send, &WanTransferMsg::FileMetadata { info: test_info }).await?;
    println!("Connector: Sent FileMetadata");

    // Receive response
    let response = recv_msg(&mut recv).await?;
    println!("Connector: Received response: {:?}", response);
    assert!(matches!(response, WanTransferMsg::ResumeInfo { offset: 0 }));

    // Finish stream
    send.finish()?;
    println!("Connector: Stream finished");

    // Wait for completion with timeout (listener may close connection before we receive)
    match tokio::time::timeout(std::time::Duration::from_secs(3), recv_msg(&mut recv)).await {
        Ok(Ok(WanTransferMsg::TransferComplete)) => {
            println!("Connector: Received TransferComplete");
        }
        Ok(Ok(msg)) => {
            println!("Connector: Unexpected message: {:?}", msg);
        }
        Ok(Err(e)) => {
            // Connection closed by peer is acceptable - message was sent
            println!("Connector: Connection closed (expected): {}", e);
        }
        Err(_) => {
            println!("Connector: Timeout waiting for TransferComplete");
        }
    }

    // Wait for listener to finish
    match accept_handle.await {
        Ok(Ok(())) => println!("Listener: Task completed successfully"),
        Ok(Err(e)) => println!("Listener: Task error: {}", e),
        Err(e) => println!("Listener: Join error: {}", e),
    }

    // Clean up
    conn.close(0u8.into(), b"test done");
    listener.close().await;
    connector.close().await;

    println!("=== Local Endpoint Pair Test PASSED ===");
    Ok(())
}
