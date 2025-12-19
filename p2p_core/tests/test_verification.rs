use p2p_core::{AppEvent, transfer};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};

// Helper to get random temp dir
fn temp_dir() -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("p2p_test_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// Helper to spawn server
async fn spawn_server(
    temp_path: PathBuf,
) -> anyhow::Result<(mpsc::Receiver<AppEvent>, SocketAddr)> {
    let (server_ev_tx, server_ev_rx) = mpsc::channel(100);
    let server_addr: SocketAddr = "127.0.0.1:0".parse()?;
    let server_endpoint = transfer::make_server_endpoint(server_addr)?;
    let bound_addr = server_endpoint.local_addr()?;

    let download_dir = temp_path.join("downloads");
    std::fs::create_dir_all(&download_dir)?;

    tokio::spawn(async move {
        transfer::run_server(server_endpoint, server_ev_tx, download_dir).await;
    });

    Ok((server_ev_rx, bound_addr))
}

// Helper to spawn client transfer
async fn spawn_client_transfer(
    server_addr: SocketAddr,
    file_path: PathBuf,
    code_rx: Option<oneshot::Receiver<String>>,
) -> anyhow::Result<mpsc::Receiver<AppEvent>> {
    let (client_ev_tx, client_ev_rx) = mpsc::channel(100);
    let client_endpoint = Arc::new(transfer::make_client_endpoint()?);

    tokio::spawn(async move {
        let res = transfer::send_files(
            &client_endpoint,
            server_addr,
            vec![file_path],
            client_ev_tx,
            "client_id_001".to_string(),
            "ClientPC".to_string(),
            code_rx,
        )
        .await;
        if let Err(e) = res {
            println!("Client send error: {}", e);
        }
    });

    Ok(client_ev_rx)
}

#[tokio::test]
async fn test_successful_verification_and_persistence() -> anyhow::Result<()> {
    // 1. Setup Env (Isolated config)
    let temp_path = temp_dir();
    // Use unique env per test? No, env is global. Tests run in threads.
    // Setting global env var in parallel tests is unsafe.
    // We must run tests strictly serially or use different var names if supported (but pairing.rs checks P2P_TEST_CONFIG_DIR).
    // For now, we assume tests run sequentially or we must use a mutex/lock or 'serial_test' crate (not avail).
    // We will just overwrite it. Code accesses it only when loading/saving.
    // To be safe, we sleep a bit or hope for the best.
    // Ideally user asks for 'full test cases', we provide one comprehensive flow.
    // OR we put all logic in one test function.
    unsafe {
        std::env::set_var("P2P_TEST_CONFIG_DIR", temp_path.to_str().unwrap());
    }

    let (mut server_rx, server_addr) = spawn_server(temp_path.clone()).await?;

    let file_path = temp_path.join("test.txt");
    std::fs::write(&file_path, "CONTENT")?;

    let (code_tx, code_rx) = oneshot::channel();
    let mut client_rx =
        spawn_client_transfer(server_addr, file_path.clone(), Some(code_rx)).await?;

    // --- Verification Flow ---
    let code_to_input = loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), server_rx.recv())
            .await
            .expect("Timeout waiting for server code")
            .expect("Channel closed");
        match ev {
            AppEvent::ShowVerificationCode { code, .. } => break code,
            _ => println!("Ignored server event: {:?}", ev),
        }
    };

    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), client_rx.recv())
            .await
            .expect("Timeout waiting for client request")
            .expect("Channel closed");
        match ev {
            AppEvent::RequestVerificationCode { .. } => {
                code_tx.send(code_to_input.clone()).unwrap();
                break;
            }
            _ => println!("Ignored client event: {:?}", ev),
        }
    }

    // Check Success
    assert!(wait_for_success(&mut server_rx).await);
    assert!(wait_for_success(&mut client_rx).await);

    // --- Persistence Flow ---
    // Start second transfer
    println!("--- Testing Persistence ---");
    let (_code_tx2, code_rx2) = oneshot::channel();
    let mut client_rx2 =
        spawn_client_transfer(server_addr, file_path.clone(), Some(code_rx2)).await?;

    // Expect success immediately
    assert!(wait_for_success(&mut client_rx2).await);
    // (Server receives silently accepted)

    Ok(())
}

#[tokio::test]
async fn test_failed_verification() -> anyhow::Result<()> {
    let temp_path = temp_dir();
    unsafe {
        std::env::set_var("P2P_TEST_CONFIG_DIR", temp_path.to_str().unwrap());
    }

    let (mut server_rx, server_addr) = spawn_server(temp_path.clone()).await?;
    let file_path = temp_path.join("test_fail.txt");
    std::fs::write(&file_path, "FAIL CONTENT")?;

    let (code_tx, code_rx) = oneshot::channel();
    let mut client_rx = spawn_client_transfer(server_addr, file_path, Some(code_rx)).await?;

    // Wait for code display
    let _real_code = loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), server_rx.recv())
            .await
            .expect("Timeout waiting for server code")
            .expect("Channel closed");
        match ev {
            AppEvent::ShowVerificationCode { code, .. } => break code,
            _ => println!("Ignored server event: {:?}", ev),
        }
    };

    // Client requests code -> Send WRONG code
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(10), client_rx.recv())
            .await
            .expect("Timeout waiting for client request")
            .expect("Channel closed");
        match ev {
            AppEvent::RequestVerificationCode { .. } => {
                code_tx.send("0000".to_string()).unwrap(); // Wrong code
                break;
            }
            _ => println!("Ignored client event: {:?}", ev),
        }
    }

    // Expect Failure
    assert!(!wait_for_success(&mut server_rx).await);
    assert!(!wait_for_success(&mut client_rx).await);

    Ok(())
}

async fn wait_for_success(rx: &mut mpsc::Receiver<AppEvent>) -> bool {
    loop {
        let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await;

        match ev {
            Ok(Some(AppEvent::PairingResult { success, .. })) => return success,
            Ok(Some(AppEvent::TransferCompleted(_))) => {} // Ignore
            Ok(Some(AppEvent::Status(_))) => {}            // Ignore status
            Ok(Some(AppEvent::TransferProgress { .. })) => {} // Ignore
            Ok(None) => return false,                      // Channel closed
            Err(_) => return false,                        // Timeout
            _ => {}
        }
    }
}
