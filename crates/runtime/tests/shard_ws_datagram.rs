//! Wire version 3 over the shard WebSocket: replication updates as
//! datagram frames, acks back from the client.
//!
//! The client drops a fifth of the datagrams it receives, as a lossy link
//! would, and acks the rest. The units move and change hit points; once
//! they hold still, the client's table must match the store exactly.

use std::net::{TcpListener, TcpStream};
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
use tungstenite::client::IntoClientRequest;
use tungstenite::Message;

const PRECISION: f32 = 0.01;
const HP: u8 = 1;

struct Field {
    store: Replicated,
    t: f32,
    frozen: Arc<AtomicBool>,
}

impl SimState for Field {
    /// (unit, hit points)
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
    fn tick(&mut self, dt: std::time::Duration) {
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

struct Rng(u64);
impl Rng {
    fn chance(&mut self, percent: u64) -> bool {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % 100 < percent
    }
}

#[test]
fn version_3_converges_through_dropped_datagrams() {
    const DMAX: usize = 200;
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
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let dyn_registry: Arc<dyn DynShardRegistry> = registry.clone();
    let s = Arc::clone(&sessions);
    std::thread::spawn(move || pylon_runtime::shard_ws::serve(listener, dyn_registry, s, 0));

    let token = sessions.create("c1".to_string()).token;
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!("ws://127.0.0.1:{port}/?shard=field&sid=c1&v=3&dmax={DMAX}")
        .into_client_request()
        .unwrap();
    req.headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (mut ws, _) = tungstenite::client(req, stream).expect("handshake");
    ws.get_ref()
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();

    let mut table = ReplicaTable::new();
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let (mut datagrams, mut dropped, mut stream_frames) = (0u32, 0u32, 0u32);
    let mut pending_acks: Vec<(u64, u64)> = Vec::new();
    let started = Instant::now();
    let mut sent_input = false;
    let mut frozen_at: Option<Instant> = None;
    loop {
        let elapsed = started.elapsed();
        if !sent_input && elapsed > Duration::from_millis(500) {
            // An input: type byte 0, then the envelope in the shard's codec.
            let mut msg = vec![wire::client::INPUT];
            msg.extend(serde_json::to_vec(&serde_json::json!({ "input": [7, 42] })).unwrap());
            ws.send(Message::Binary(msg.into())).unwrap();
            sent_input = true;
        }
        if frozen_at.is_none() && elapsed > Duration::from_secs(3) {
            frozen.store(true, Ordering::Release);
            frozen_at = Some(Instant::now());
        }
        // Lossy while moving; everything once frozen, so it converges.
        let lossy = frozen_at.is_none();
        if frozen_at.is_some_and(|t| t.elapsed() > Duration::from_millis(1500)) {
            break;
        }
        match ws.read() {
            Ok(Message::Binary(bytes)) => {
                assert!(bytes.len() >= wire::HEADER_LEN);
                let (k, payload) = (bytes[0], &bytes[wire::HEADER_LEN..]);
                match k {
                    kind::REPLICATION => {
                        table.apply(payload).expect("a stream frame applies");
                        stream_frames += 1;
                    }
                    kind::DATAGRAM => {
                        assert!(
                            payload.len() <= DMAX,
                            "a datagram of {} bytes",
                            payload.len()
                        );
                        datagrams += 1;
                        if lossy && rng.chance(20) {
                            dropped += 1;
                            continue;
                        }
                        let s = table.apply_datagram(payload).expect("a datagram decodes");
                        pending_acks.push((s.frame, table.frames_applied));
                    }
                    other => panic!("unexpected frame kind {other}"),
                }
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => panic!("read: {e}"),
        }
        if pending_acks.len() >= 8 || (!pending_acks.is_empty() && !lossy) {
            let msg = wire::encode_datagram_acks(&pending_acks);
            ws.send(Message::Binary(msg.into())).unwrap();
            pending_acks.clear();
        }
    }
    assert!(
        stream_frames >= 1,
        "the first frame is a full frame on the stream"
    );
    assert!(datagrams > 100, "{datagrams} datagrams");
    assert!(dropped > 10, "{dropped} dropped");

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
