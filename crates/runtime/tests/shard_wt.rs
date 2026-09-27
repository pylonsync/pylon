//! Shard sessions over WebTransport, end to end with a real QUIC client.
//!
//! The client pins the server's self-signed certificate by its hash (as a
//! browser does with `serverCertificateHashes`), sends its hello on a
//! stream, gets its baseline on the stream and its updates as datagrams,
//! drops a fifth of those, acks the rest with datagrams, and sends an input.
//! Once the units hold still, its table matches the store. A hello with a
//! bad ticket is refused with the reason.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_auth::SessionStore;
use pylon_realtime::replicated_store::frame::quantize3;
use pylon_realtime::wire::{self, kind};
use pylon_realtime::{
    DynShardRegistry, EntityId, Plane, ReplicaTable, Replicated, ReplicatedRef, ReplicationConfig,
    Shard, ShardConfig, ShardRegistry, SimState, SubscriberId,
};
use pylon_runtime::shard_wt::{self, WebTransportConfig};
use wtransport::error::ConnectionError;
use wtransport::tls::Sha256Digest;
use wtransport::{ClientConfig, Connection, Endpoint, RecvStream, SendStream};

const PRECISION: f32 = 0.01;
const HP: u8 = 1;

struct Field {
    store: Replicated,
    t: f32,
    frozen: Arc<AtomicBool>,
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
    fn tick(&mut self, dt: Duration) {
        if self.frozen.load(Ordering::Acquire) {
            return;
        }
        self.t += dt.as_secs_f32();
        let ids: Vec<EntityId> = self.store.iter().map(|(id, _)| id).collect();
        for id in ids {
            let a = self.t * 3.0 + id as f32;
            self.store
                .set_pos(id, [id as f32 * 3.0 + a.cos() * 2.0, a.sin() * 2.0, 0.0]);
        }
    }
    fn snapshot(&self) {}
    fn replicated(&self) -> Option<ReplicatedRef<'_>> {
        Some((&self.store).into())
    }
    fn replication_config(&self) -> ReplicationConfig {
        ReplicationConfig {
            precision: PRECISION,
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

async fn connect(url: &str, hashes: &[[u8; 32]]) -> Connection {
    let config = ClientConfig::builder()
        .with_bind_default()
        .with_server_certificate_hashes(hashes.iter().map(|h| Sha256Digest::new(*h)))
        .build();
    Endpoint::client(config)
        .unwrap()
        .connect(url)
        .await
        .expect("a WebTransport session")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_converges_through_dropped_datagrams() {
    let frozen = Arc::new(AtomicBool::new(false));
    let mut store = Replicated::new();
    for id in 0..40 {
        store.spawn(id, [id as f32 * 3.0, 0.0, 0.0]);
        store.set_component(id, HP, &[100]);
    }
    let registry: Arc<ShardRegistry<Field>> = Arc::new(ShardRegistry::new());
    registry.insert(Shard::new(
        "field",
        Field {
            store,
            t: 0.0,
            frozen: Arc::clone(&frozen),
        },
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
        Arc::clone(&sessions),
    )
    .expect("WebTransport starts");

    // What a browser fetches from the app first.
    let (info_url, hashes) = shard_wt::endpoint_info().unwrap();
    assert_eq!(info_url, url);
    assert_eq!(hashes.len(), 2, "the current and the next certificate");
    let json: serde_json::Value =
        serde_json::from_str(&shard_wt::endpoint_info_json().unwrap()).unwrap();
    assert_eq!(json["certHashes"].as_array().unwrap().len(), 2);

    // A bad ticket is refused with the reason.
    let refused = connect(&url, &hashes).await;
    let (mut send, _recv) = refused.open_bi().await.unwrap().await.unwrap();
    write(
        &mut send,
        br#"{"shard":"field","sid":"c2","ticket":"v1.bad.bad"}"#,
    )
    .await;
    match tokio::time::timeout(Duration::from_secs(5), refused.closed()).await {
        Ok(ConnectionError::ApplicationClosed(close)) => {
            let reason = String::from_utf8_lossy(close.reason()).into_owned();
            assert!(reason.starts_with("unauthorized"), "{reason}");
            assert_eq!(
                close.code().into_inner(),
                u64::from(shard_wt::CLOSE_CODES.1)
            );
        }
        other => panic!("expected an application close, got {other:?}"),
    }

    let token = sessions.create("c1".to_string()).token;
    let conn = connect(&url, &hashes).await;
    let (mut send, mut recv) = conn.open_bi().await.unwrap().await.unwrap();
    write(
        &mut send,
        serde_json::json!({ "shard": "field", "sid": "c1", "token": token })
            .to_string()
            .as_bytes(),
    )
    .await;

    // The stream: frames, in order, on their own task.
    let (frames_tx, mut frames) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(f) = read(&mut recv).await {
            if frames_tx.send(f).is_err() {
                break;
            }
        }
    });

    let mut table = ReplicaTable::new();
    let mut skip = 0x2545_f491_4f6c_dd1du64;
    let (mut datagrams, mut dropped, mut stream_frames) = (0u32, 0u32, 0u32);
    let max = conn.max_datagram_size().unwrap();
    let started = Instant::now();
    let mut sent_input = false;
    let mut frozen_at: Option<Instant> = None;
    loop {
        let elapsed = started.elapsed();
        if !sent_input && elapsed > Duration::from_millis(500) {
            let mut msg = vec![wire::client::JSON_INPUT];
            msg.extend(serde_json::to_vec(&serde_json::json!({ "input": [7, 42] })).unwrap());
            write(&mut send, &msg).await;
            sent_input = true;
        }
        if frozen_at.is_none() && elapsed > Duration::from_secs(3) {
            frozen.store(true, Ordering::Release);
            frozen_at = Some(Instant::now());
        }
        if frozen_at.is_some_and(|t| t.elapsed() > Duration::from_millis(1500)) {
            break;
        }
        let lossy = frozen_at.is_none();
        tokio::select! {
            Some(frame) = frames.recv() => {
                assert_eq!(frame[0], kind::REPLICATION, "a stream frame");
                table.apply(&frame[wire::HEADER_LEN..]).expect("a stream frame applies");
                stream_frames += 1;
            }
            d = conn.receive_datagram() => {
                let d = d.expect("a datagram");
                assert!(d.len() <= max);
                datagrams += 1;
                skip ^= skip << 13;
                skip ^= skip >> 7;
                skip ^= skip << 17;
                if lossy && skip % 5 == 0 {
                    dropped += 1;
                    continue;
                }
                let s = table.apply_datagram(&d).expect("a datagram decodes");
                conn.send_datagram(wire::encode_datagram_acks(&[(s.frame, table.frames_applied)]))
                    .unwrap();
            }
            _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        }
    }
    assert!(stream_frames >= 1);
    // About one datagram per tick: 40 units' updates fit in one.
    assert!(datagrams > 40, "{datagrams} datagrams");
    assert!(dropped > 3, "{dropped} dropped");

    let truth: Vec<(EntityId, [i64; 3], Option<Vec<u8>>)> =
        registry.get("field").unwrap().with_state(|f| {
            f.store
                .iter()
                .map(|(id, e)| {
                    (
                        id,
                        quantize3(e.pos, PRECISION),
                        e.component(HP).map(<[u8]>::to_vec),
                    )
                })
                .collect()
        });
    assert_eq!(table.entities.len(), truth.len());
    for (id, q, hp) in truth {
        let e = &table.entities[&id];
        assert_eq!(e.q, q, "position of {id}");
        assert_eq!(e.components.get(&HP).cloned(), hp, "hp of {id}");
    }
    assert_eq!(
        table.entities[&7].components[&HP],
        vec![42],
        "the input applied"
    );
}
