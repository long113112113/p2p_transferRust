---
description: Document
---

# Tài liệu Iroh 0.95.1

## Giới thiệu

**Iroh** là thư viện Rust để thiết lập kết nối peer-to-peer QUIC trực tiếp giữa các thiết bị. Iroh tự động xử lý NAT traversal, hole-punching, và sử dụng relay server khi cần thiết.

## Breaking Changes từ phiên bản cũ đến 0.95.1

### 1. Thay đổi cấu trúc module
- **Trước đây**: `use iroh::net::Endpoint;`
- **Bây giờ**: `use iroh::Endpoint;`
- Module `net` đã bị loại bỏ, các type được export trực tiếp từ root crate

### 2. Đổi tên types và methods

#### NodeId → EndpointId
- `iroh_base::NodeId` → `iroh::EndpointId`
- `iroh_base::NodeAddr` → `iroh::EndpointAddr`
- `iroh_base::NodeTicket` → `iroh::EndpointTicket`

#### Methods của Endpoint
- `endpoint.node_id()` → `endpoint.id()`
- `endpoint.node_addr()` → `endpoint.addr()`
- `endpoint.watch_node_addr()` → `endpoint.watch_addr()`
- `endpoint.listen_addr()` → `endpoint.addr()` (không còn hỗ trợ `listen_addr`)

#### Methods của Connection
- `conn.remote_node_id()?` → `conn.remote_id()` (không còn trả về Result)
- Connection methods trở thành **infallible** - không còn trả về Result cho remote_id và alpn

### 3. Thay đổi về Connection Accept

**Trước đây** (API cũ):
```rust
while let Some(incoming) = endpoint.accept().await {
    let conn = incoming.accept()?.await?;  // 2 phép toán: accept() và await
}
```

**Bây giờ** (0.95.1):
```rust
while let Some(connecting) = endpoint.accept().await {
    let conn = connecting.await?;  // Chỉ cần await
}
```

## Các API chính

### 1. Endpoint - Điểm vào chính

```rust
use iroh::{Endpoint, EndpointId, Watcher};

// Tạo endpoint với configuration mặc định
let endpoint = Endpoint::builder()
    .alpns(vec![b"my-protocol".to_vec()])
    .bind()
    .await?;

// Lấy ID của endpoint (để peer khác kết nối)
let my_id = endpoint.id();

// Lấy địa chỉ hiện tại (bao gồm relay và direct addresses)
let addr = endpoint.addr();

// Watch để biết khi địa chỉ thay đổi
let mut addr_watcher = endpoint.watch_addr();
let current_addr = addr_watcher.get();
```

### 2. Kết nối đến Peer khác

```rust
use std::str::FromStr;

// Parse EndpointId từ string
let peer_id = EndpointId::from_str("02ab2b...")?;

// Kết nối
let conn = endpoint.connect(peer_id, b"my-protocol").await?;

// Lấy thông tin về peer
let remote = conn.remote_id();
let alpn = conn.alpn();
```

### 3. Chấp nhận kết nối đến

```rust
while let Some(connecting) = endpoint.accept().await {
    let conn = connecting.await?;
    println!("Peer connected: {}", conn.remote_id());
    
    // Xử lý connection trong task riêng
    tokio::spawn(async move {
        handle_connection(conn).await;
    });
}
```

### 4. Lấy thông tin về kết nối

```rust
use iroh::Watcher;

// Loại kết nối (Direct hoặc Relay)
if let Some(mut conn_type_watcher) = endpoint.conn_type(peer_id) {
    let conn_type = conn_type_watcher.get();
    println!("Connection type: {:?}", conn_type);
}

// RTT (Round-Trip Time)
let rtt = conn.rtt();
println!("Latency: {:?}", rtt);

// Statistics
let stats = conn.stats();
println!("TX datagrams: {}", stats.udp_tx.datagrams);
println!("RX datagrams: {}", stats.udp_rx.datagrams);
println!("TX bytes: {}", stats.udp_tx.bytes);
println!("RX bytes: {}", stats.udp_rx.bytes);
println!("Path info: {:?}", stats.path);
```

### 5. Real-time Connection Monitoring

For continuous monitoring of connection statistics, spawn a background task that periodically queries connection info:

```rust
use std::time::Duration;
use tokio::time;

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

        let stats = conn.stats();
        let elapsed = start_time.elapsed().as_secs_f64();

        // Calculate throughput (bytes/sec since last update)
        let tx_delta = stats.udp_tx.bytes.saturating_sub(prev_stats.udp_tx.bytes);
        let rx_delta = stats.udp_rx.bytes.saturating_sub(prev_stats.udp_rx.bytes);
        let tx_throughput = tx_delta as f64 / 2.0; // interval duration
        let rx_throughput = rx_delta as f64 / 2.0;

        println!("\n{}", "=".repeat(60));
        println!("📊 Connection Stats Update (t={:.1}s)", elapsed);
        println!("{}", "=".repeat(60));

        // Connection type (can change from Relay to Direct)
        if let Some(mut conn_type_watcher) = endpoint.conn_type(peer_id) {
            println!("📡 Type: {:?}", conn_type_watcher.get());
        }

        // RTT (may fluctuate)
        println!("⏱️  RTT: {:?}", conn.rtt());

        // Throughput
        println!("\n📈 Throughput:");
        println!("   TX: {:.2} bytes/s ({} total)", tx_throughput, stats.udp_tx.bytes);
        println!("   RX: {:.2} bytes/s ({} total)", rx_throughput, stats.udp_rx.bytes);

        // Path quality metrics
        println!("\n🛣️  Path Stats:");
        println!("   CWND: {} bytes", stats.path.cwnd);
        println!("   Lost packets: {}", stats.path.lost_packets);
        println!("   Current MTU: {}", stats.path.current_mtu);

        prev_stats = stats;
    }
}

// Spawn monitoring task
tokio::spawn(monitor_connection_info(
    endpoint.clone(),
    conn.clone(),
    peer_id,
));
```

**Key Monitoring Metrics**:
- **Connection Type**: Track upgrades from Relay → Direct
- **RTT**: Monitor latency changes in real-time
- **Throughput**: Calculate actual data transfer rates
- **Packet Loss**: Detect network quality degradation
- **CWND** (Congestion Window): Observe congestion control behavior
- **MTU**: Current Maximum Transmission Unit


## Connection Types

### Direct Connection
- Kết nối UDP trực tiếp giữa hai peers
- Độ trễ thấp nhất
- Iroh ưu tiên loại này và sẽ tự động thực hiện hole-punching

### Relay Connection
- Kết nối qua relay server (DERP)
- Được dùng khi:
  - Không thể thiết lập direct connection
  - Đang trong quá trình hole-punching
  - Firewall/NAT quá nghiêm ngặt
- Tất cả traffic đều được mã hóa end-to-end (relay không thể đọc được)
- Iroh sẽ tự động chuyển sang direct khi có thể

## Connection Stats

### ConnectionStats struct
```rust
pub struct ConnectionStats {
    pub udp_tx: UdpStats,      // UDP transmit stats
    pub udp_rx: UdpStats,      // UDP receive stats
    pub frame_tx: FrameStats,  // Frame transmit stats
    pub frame_rx: FrameStats,  // Frame receive stats
    pub path: PathStats,       // Path information
}
```

### UdpStats
- `datagrams`: Số lượng UDP datagrams
- `bytes`: Tổng số bytes
- `ios`: Số lượng I/O operations

## Streaming API

### Mở stream mới

```rust
// Unidirectional stream
let mut send = conn.open_uni().await?;
send.write_all(b"Hello").await?;
send.finish()?;

// Bidirectional stream
let (mut send, mut recv) = conn.open_bi().await?;
send.write_all(b"Hello").await?;
let response = recv.read_to_end(1024).await?;
```

### Chấp nhận stream

```rust
// Accept unidirectional
while let Some(mut recv) = conn.accept_uni().await? {
    let data = recv.read_to_end(1024).await?;
}

// Accept bidirectional
while let Some((mut send, mut recv)) = conn.accept_bi().await? {
    let data = recv.read_to_end(1024).await?;
    send.write_all(b"Response").await?;
}
```

## Datagrams

```rust
// Gửi datagram (unreliable, unordered)
let data = bytes::Bytes::from("Hello");
conn.send_datagram(data)?;

// Nhận datagram
let data = conn.read_datagram().await?;

// Kiểm tra kích thước tối đa
if let Some(max_size) = conn.max_datagram_size() {
    println!("Max datagram size: {}", max_size);
}
```

## Online Status

```rust
// Đợi endpoint "online" (đã kết nối relay)
endpoint.online().await;

// Watch để biết khi endpoint offline
let mut addr_watcher = endpoint.watch_addr();
tokio::spawn(async move {
    while addr_watcher.changed().await.is_ok() {
        let addr = addr_watcher.borrow();
        if addr.addrs.is_empty() {
            println!("Endpoint is offline");
        }
    }
});
```

## Error Handling

### Connection Errors
- `ConnectionError::LocallyClosed` - Đóng từ phía local
- `ConnectionError::ApplicationClosed` - Application đóng connection
- `ConnectionError::Reset` - Connection bị reset
- `ConnectionError::TimedOut` - Timeout

## Best Practices

### 1. ALPN (Application-Layer Protocol Negotiation)
- Luôn set ALPN để accept incoming connections
- Peers phải dùng cùng ALPN protocol
- Có thể dùng nhiều ALPNs và kiểm tra bằng `Connecting::alpn()`

### 2. Graceful Shutdown
```rust
// Đóng endpoint và đợi connections cleanup
endpoint.close(0u32.into(), b"shutdown").await?;
```

### 3. Connection Management
- Sử dụng một Endpoint instance cho toàn bộ application
- Clone Connection để share giữa nhiều tasks
- Connection tự động cleanup khi dropped

### 4. Resource Limits
```rust
// Kiểm tra buffer space trước khi gửi datagram
let available = conn.datagram_send_buffer_space();
if available >= data.len() {
    conn.send_datagram(data)?;
}
```

## Debugging

### Enable logging
```toml
[dependencies]
tracing-subscriber = "0.3"
```

```rust
tracing_subscriber::fmt()
    .with_env_filter("iroh=debug")
    .init();
```

### Connection Metrics
```rust
// Endpoint metrics
let metrics = endpoint.metrics();

// Connection congestion state
let congestion = conn.congestion_state();
println!("Congestion state: {:?}", congestion);
```

## Common Issues và Solutions

### Issue 1: "could not find `net` in `iroh`"
**Solution**: Update import từ `iroh::net::Endpoint` thành `iroh::Endpoint`

### Issue 2: "no method named `node_id`"
**Solution**: Đổi `.node_id()` thành `.id()`

### Issue 3: "EndpointAddr doesn't implement Display"
**Solution**: Dùng `{:?}` thay vì `{}` khi format

### Issue 4: "no method named `get`"
**Solution**: Import `iroh::Watcher` trait và sử dụng `mut` cho watcher:
```rust
use iroh::Watcher;
if let Some(mut watcher) = endpoint.conn_type(peer_id) {
    let value = watcher.get();
}
```

### Issue 5: Connection timeout
**Solution**: 
- Đảm bảo relay server đang hoạt động
- Đợi `endpoint.online().await` trước khi kết nối
- Kiểm tra firewall settings

## Links tham khảo

- [Iroh Documentation](https://docs.rs/iroh/0.95.1/)
- [Iroh GitHub](https://github.com/n0-computer/iroh)
- [Iroh Website](https://iroh.computer/)
- [Release Notes 0.95.1](https://github.com/n0-computer/iroh/releases/tag/v0.95.1)

## Ví dụ hoàn chỉnh

Xem file `src/main.rs` trong project này để có ví dụ hoàn chỉnh về:
- Thiết lập endpoint
- Client/Server mode
- Connection info logging
- RTT measurement
- Connection type detection


## Identity Management

### SecretKey
To keep the Node ID persistent (stable across restarts), you need to manage `SecretKey`.

#### Dependencies
```toml
[dependencies]
iroh = "0.95.1"
rand = "0.9"  # Required - iroh does not re-export rand
```

#### Generate and Persist SecretKey

```rust
use iroh::SecretKey;

// Generate new key - use rand::rng() to avoid rand_core version conflicts
let secret_key = SecretKey::generate(&mut rand::rng());

// Serialize to bytes (32 bytes)
let bytes: [u8; 32] = secret_key.to_bytes();
// Save `bytes` to file/db...

// Load from bytes
let loaded_key = SecretKey::from_bytes(&bytes);

// Use in Endpoint
let endpoint = Endpoint::builder()
    .secret_key(loaded_key)
    .bind()
    .await?;
```

#### Complete Example (IdentityManager)

```rust
use anyhow::{Context, Result};
use iroh::SecretKey;
use std::path::PathBuf;
use tokio::fs;

const KEY_FILE_NAME: &str = "node_secret.key";

pub struct IdentityManager {
    config_dir: PathBuf,
}

impl IdentityManager {
    pub fn new(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    pub async fn load_or_generate(&self) -> Result<SecretKey> {
        let key_path = self.config_dir.join(KEY_FILE_NAME);

        if key_path.exists() {
            // Load existing key
            let key_bytes = fs::read(&key_path)
                .await
                .context("Failed to read secret key file")?;
            let bytes: [u8; 32] = key_bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("Invalid secret key length in file"))?;
            Ok(SecretKey::from_bytes(&bytes))
        } else {
            // Generate new key
            let secret_key = SecretKey::generate(&mut rand::rng());
            
            if let Some(parent) = key_path.parent() {
                fs::create_dir_all(parent)
                    .await
                    .context("Failed to create config directory")?;
            }

            fs::write(&key_path, secret_key.to_bytes())
                .await
                .context("Failed to save secret key")?;

            Ok(secret_key)
        }
    }
}
```

#### Key Points

- `EndpointId` is the `PublicKey` corresponding to the `SecretKey`
- `EndpointId` remains unchanged as long as `SecretKey` is preserved
- Use `secret_key.public()` to get the corresponding `PublicKey`/`EndpointId`

### Issue 6: `OsRng: CryptoRng` trait bound not satisfied

**Error**:
```
the trait bound `OsRng: CryptoRng` is not satisfied
there are multiple different versions of crate `rand_core` in the dependency graph
```

**Cause**: Version conflict between `rand_core` used by your `rand` crate and the one used internally by iroh.

**Solution**: Use `rand::rng()` instead of `rand::rngs::OsRng`:

```rust
// ❌ Wrong - may cause rand_core version conflict
let mut rng = rand::rngs::OsRng;
let secret_key = SecretKey::generate(&mut rng);

// ✅ Correct - official iroh recommendation
let secret_key = SecretKey::generate(&mut rand::rng());
```

`rand::rng()` is the new API in rand 0.9+ (replaces `thread_rng()`) and internally uses `OsRng` while avoiding version conflicts.
