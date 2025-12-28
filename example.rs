use anyhow::Result;
use iroh::{Endpoint, EndpointId, Watcher};
use std::env;
use std::str::FromStr;
use std::time::Duration;
use tokio::time;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    // 1. Khởi tạo Endpoint - Iroh sẽ tự động lo phần NAT Traversal/DERP
    let endpoint = Endpoint::builder()
        .alpns(vec![b"iroh-test-protocol".to_vec()]) // Định nghĩa giao thức tạm thời
        .bind()
        .await?;

    let node_id = endpoint.id();
    println!("cargo run -- {}", node_id);
    println!("Đang chờ kết nối hoặc chuẩn bị kết nối...");

    if args.len() > 1 {
        // CHẾ ĐỘ CLIENT: Kết nối tới Node ID được cung cấp
        let peer_id = EndpointId::from_str(&args[1])?;
        println!("Đang cố gắng kết nối tới: {}", peer_id);

        // Iroh sẽ thử đục lỗ UDP, nếu không được sẽ tự qua Relay (DERP)
        let conn = endpoint.connect(peer_id, b"iroh-test-protocol").await?;
        println!("✅ Đã kết nối thành công tới {}", peer_id);

        // Monitor thông tin kết nối theo thời gian thực
        let monitor_task = tokio::spawn(monitor_connection_info(
            endpoint.clone(),
            conn.clone(),
            peer_id,
        ));

        // Giữ kết nối
        tokio::signal::ctrl_c().await?;
        monitor_task.abort();
    } else {

        while let Some(connecting) = endpoint.accept().await {
            let conn = connecting.await?;
            let remote_id = conn.remote_id();
            println!("\n✅ Có thiết bị vừa kết nối tới: {}", remote_id);

            // Spawn task để monitor connection này
            let endpoint_clone = endpoint.clone();
            tokio::spawn(monitor_connection_info(
                endpoint_clone,
                conn.clone(),
                remote_id,
            ));
        }
    }

    Ok(())
}

async fn monitor_connection_info(
    endpoint: Endpoint,
    conn: iroh::endpoint::Connection,
    peer_id: EndpointId,
) {
    let mut interval = time::interval(Duration::from_secs(2));
    let mut prev_stats = conn.stats();
    let start_time = std::time::Instant::now();

    loop {
        interval.tick().await;

        // Lấy stats hiện tại
        let stats = conn.stats();
        let elapsed = start_time.elapsed().as_secs_f64();

        // Tính throughput (bytes/sec từ lần update trước)
        let tx_delta = stats.udp_tx.bytes.saturating_sub(prev_stats.udp_tx.bytes);
        let rx_delta = stats.udp_rx.bytes.saturating_sub(prev_stats.udp_rx.bytes);
        let tx_throughput = tx_delta as f64 / 2.0; // chia cho interval duration
        let rx_throughput = rx_delta as f64 / 2.0;

        // Print header với timestamp
        println!("\n{}", "=".repeat(60));
        println!("📊 Connection Stats Update (t={:.1}s)", elapsed);
        println!("{}", "=".repeat(60));

        // Loại kết nối
        if let Some(mut conn_type_watcher) = endpoint.conn_type(peer_id) {
            let conn_type = conn_type_watcher.get();
            println!("📡 Connection Type: {:?}", conn_type);
        }

        // RTT
        let rtt = conn.rtt();
        println!("⏱️  RTT: {:?}", rtt);

        // Throughput
        println!("\n📈 Throughput:");
        println!(
            "   TX: {:.2} bytes/s ({} bytes total)",
            tx_throughput, stats.udp_tx.bytes
        );
        println!(
            "   RX: {:.2} bytes/s ({} bytes total)",
            rx_throughput, stats.udp_rx.bytes
        );

        // UDP Stats
        println!("\n� UDP Packets:");
        println!(
            "   TX: {} datagrams ({} IOs)",
            stats.udp_tx.datagrams, stats.udp_tx.ios
        );
        println!(
            "   RX: {} datagrams ({} IOs)",
            stats.udp_rx.datagrams, stats.udp_rx.ios
        );

        // Path stats
        println!("\n🛣️  Path Stats:");
        println!("   RTT: {:?}", stats.path.rtt);
        println!("   CWND: {} bytes", stats.path.cwnd);
        println!("   Lost packets: {}", stats.path.lost_packets);
        println!("   Lost bytes: {}", stats.path.lost_bytes);
        println!("   Sent packets: {}", stats.path.sent_packets);
        println!("   Current MTU: {}", stats.path.current_mtu);

        // Update prev_stats cho lần sau
        prev_stats = stats;
    }
}
