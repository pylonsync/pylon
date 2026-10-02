//! An input whose bytes arrive in two parts, with ack datagrams between
//! them, still arrives whole: a datagram must not cut a stream read short.
//!
//! Its own test binary: one WebTransport server per process.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_auth::SessionStore;
use pylon_realtime::wire::{self, kind};
use pylon_realtime::{
    DynShardRegistry, EntityId, Plane, Replicated, ReplicatedRef, ReplicationConfig, Shard,
    ShardConfig, ShardRegistry, SimState, SubscriberId,
};
use pylon_runtime::shard_wt::{self, WebTransportConfig};
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Endpoint, RecvStream, SendStream};

const HP: u8 = 1;

struct Field {
    store: Replicated,
}

impl SimState for Field {
    type Input = (EntityId, u8);
    type Snapshot = ();
    type Error = String;

    fn apply_input(
        &mut self,
        _: &SubscriberId,
        (id, hp): Self::Input,
        _: Instant,
    ) -> Result<(), String> {
        self.store.set_component(id, HP, &[hp]);
        Ok(())
    }
    fn tick(&mut self, _: Duration) {}
    fn snapshot(&self) {}
    fn replicated(&self) -> Option<ReplicatedRef<'_>> {
        Some((&self.store).into())
    }
    fn replication_config(&self) -> ReplicationConfig {
        ReplicationConfig {
            precision: 0.01,
            max_bytes_per_tick: 0,
            plane: Plane::XY,
        }
    }
}

async fn write(send: &mut SendStream, bytes: &[u8]) {
    send.write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    send.write_all(bytes).await.unwrap();
}

async fn read(recv: &mut RecvStream) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    recv.read_exact(&mut len).await.ok()?;
    let mut buf = vec![0u8; u32::from_be_bytes(len) as usize];
    recv.read_exact(&mut buf).await.ok()?;
    Some(buf)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_split_input_survives_acks_in_between() {
    let mut store = Replicated::new();
    store.spawn(7, [0.0, 0.0, 0.0]);
    store.set_component(7, HP, &[100]);
    let registry: Arc<ShardRegistry<Field>> = Arc::new(ShardRegistry::new());
    registry.insert(Shard::new(
        "split",
        Field { store },
        ShardConfig {
            tick_rate_hz: 20,
            idle_ticks_before_shutdown: 0,
            ..Default::default()
        },
    ));
    let sessions = Arc::new(SessionStore::new());
    let port = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("https://127.0.0.1:{port}/shard");
    let dyn_registry: Arc<dyn DynShardRegistry> = registry.clone();
    shard_wt::start(
        WebTransportConfig {
            port,
            bind: Some("127.0.0.1".into()),
            url: url.clone(),
        },
        dyn_registry,
        pylon_runtime::request_auth::AuthResolver::from_sessions(Arc::clone(&sessions)),
    )
    .expect("WebTransport starts");
    let (_, hashes) = shard_wt::endpoint_info().unwrap();
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes(hashes.iter().map(|h| Sha256Digest::new(*h)))
        .build();
    let conn = Endpoint::client(config)
        .unwrap()
        .connect(&url)
        .await
        .expect("a WebTransport session");
    let token = sessions.create("c9".to_string()).token;
    let (mut send, mut recv) = conn.open_bi().await.unwrap().await.unwrap();
    write(
        &mut send,
        serde_json::json!({ "shard": "split", "sid": "c9", "token": token })
            .to_string()
            .as_bytes(),
    )
    .await;
    let first = read(&mut recv).await.expect("the baseline");
    assert_eq!(first[0], kind::REPLICATION);

    let mut msg = vec![wire::client::JSON_INPUT];
    msg.extend(serde_json::to_vec(&serde_json::json!({ "input": [7, 42] })).unwrap());
    let mut framed = (msg.len() as u32).to_be_bytes().to_vec();
    framed.extend(&msg);
    // The length and a few bytes, then acks for a while, then the rest.
    send.write_all(&framed[..7]).await.unwrap();
    for _ in 0..10 {
        conn.send_datagram(wire::encode_datagram_acks(&[(1, 1)]))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    send.write_all(&framed[7..]).await.unwrap();

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let hp = registry.get("split").unwrap().with_state(|f| {
            f.store
                .get(7)
                .and_then(|e| e.component(HP).map(<[u8]>::to_vec))
        });
        if hp == Some(vec![42]) {
            break;
        }
        assert!(Instant::now() < deadline, "the input never applied");
        tokio::select! {
            e = conn.closed() => panic!("the session closed: {e:?}"),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
    }
}
