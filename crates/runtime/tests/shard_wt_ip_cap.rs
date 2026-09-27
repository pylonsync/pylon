//! WebTransport sessions count against `PYLON_SHARD_WS_MAX_PER_IP`, like
//! shard WebSockets: over the cap, a session is refused with 429 before it
//! sends a hello, and the slot frees when a session ends.
//!
//! Its own test binary: the cap is read once per process.

use std::sync::Arc;
use std::time::Duration;

use pylon_auth::SessionStore;
use pylon_realtime::DynShardRegistry;
use pylon_runtime::shard_wt::{self, WebTransportConfig};
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Connection, Endpoint, VarInt};

async fn connect(url: &str, hashes: &[[u8; 32]]) -> Result<Connection, String> {
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes(hashes.iter().map(|h| Sha256Digest::new(*h)))
        .build();
    Endpoint::client(config)
        .unwrap()
        .connect(url)
        .await
        .map_err(|e| e.to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn sessions_over_the_per_ip_cap_are_refused() {
    std::env::set_var("PYLON_SHARD_WS_MAX_PER_IP", "1");
    let registry: Arc<dyn DynShardRegistry> =
        Arc::new(pylon_realtime::ShardRegistry::<NoShard>::new());
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("https://127.0.0.1:{port}/shard");
    shard_wt::start(
        WebTransportConfig {
            port,
            bind: Some("127.0.0.1".into()),
            url: url.clone(),
        },
        registry,
        Arc::new(SessionStore::new()),
    )
    .expect("WebTransport starts");
    let (_, hashes) = shard_wt::endpoint_info().unwrap();

    let first = connect(&url, &hashes).await.expect("the first session");
    let second = connect(&url, &hashes).await;
    let err = second.err().expect("the second session is refused");
    // wtransport reports the 429 as a rejected session request.
    assert!(err.contains("rejected"), "{err}");

    // The first session ends; its slot frees.
    first.close(VarInt::from_u32(0), b"");
    let mut third = Err(String::new());
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        third = connect(&url, &hashes).await;
        if third.is_ok() {
            break;
        }
    }
    third.expect("a session after the first one ended");
}

/// A shard type for an empty registry: the cap applies before any lookup.
struct NoShard;

impl pylon_realtime::SimState for NoShard {
    type Input = ();
    type Snapshot = ();
    type Error = String;
    fn apply_input(
        &mut self,
        _: &pylon_realtime::SubscriberId,
        _: (),
        _: std::time::Instant,
    ) -> Result<(), String> {
        Ok(())
    }
    fn tick(&mut self, _: Duration) {}
    fn snapshot(&self) {}
}
