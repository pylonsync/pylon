//! Durable Object adapter for shards on Cloudflare Workers.
//!
//! The idea: one DO class per shard type. Each match/zone/document gets its
//! own DO instance (addressed by name). The DO owns the shard's state, the
//! tick loop (via `setAlarm`), and all WebSocket connections.
//!
//! Why DOs specifically: they give you exactly the isolation + persistence
//! semantics that Workers lack otherwise — single-threaded execution per
//! instance, on-instance storage, WebSocket hibernation (clients stay
//! connected even while the DO is idle and not billed).
//!
//! # Status
//!
//! This is a **scaffold**, not a working DO. The real `worker` crate has
//! a different macro story and tight coupling to wasm-bindgen. A full impl
//! requires:
//!
//! - A concrete `#[durable_object]`-annotated struct (in the user's Workers
//!   bundle, not here).
//! - A bridge from incoming WS messages → shard input queue.
//! - A bridge from shard snapshots → DO WebSocket send.
//! - `state.setAlarm()` for scheduled ticks in event-driven mode.
//! - Storage.get()/put() for persistence across hibernation.
//!
//! The abstractions below give users a head-start by providing the right
//! trait shapes for their DO implementation to hook into.

use std::sync::Arc;

use pylon_realtime::{DynShard, ShardAuth, SnapshotSink, SubscriberId};

// ---------------------------------------------------------------------------
// WorkerDoSink — bridges a shard's broadcast to DO WebSocket sends
// ---------------------------------------------------------------------------

/// Build a [`SnapshotSink`] that forwards snapshots to a DO's WebSocket send.
///
/// The caller provides the send function (which is `worker::WebSocket::send`
/// or equivalent). This lets the shard's broadcast loop push snapshots to
/// connected clients over the DO's own WebSocket.
pub fn do_websocket_sink(
    send: impl Fn(&[u8]) + Send + Sync + 'static,
) -> SnapshotSink {
    Box::new(move |tick: u64, bytes: &[u8]| {
        let mut payload = Vec::with_capacity(8 + bytes.len());
        payload.extend_from_slice(&tick.to_be_bytes());
        payload.extend_from_slice(bytes);
        send(&payload);
    })
}

// ---------------------------------------------------------------------------
// Persistence hooks for DO storage
// ---------------------------------------------------------------------------

/// Abstraction over DO storage.put()/get() so the scaffold can express the
/// persistence pattern without depending on the `worker` crate.
pub trait DoStorage: Send + Sync {
    fn get_bytes(&self, key: &str) -> Option<Vec<u8>>;
    fn put_bytes(&self, key: &str, value: &[u8]);
    fn delete(&self, key: &str);
}

/// Save a shard's serialized state to DO storage.
///
/// Called from the shard's `on_tick` hook via `persist_every_ticks`.
pub fn persist_to_do_storage<T: serde::Serialize>(
    storage: &dyn DoStorage,
    shard_id: &str,
    state: &T,
    tick: u64,
) {
    if let Ok(bytes) = serde_json::to_vec(state) {
        storage.put_bytes(&format!("shard:{shard_id}:state"), &bytes);
        storage.put_bytes(
            &format!("shard:{shard_id}:tick"),
            &tick.to_be_bytes(),
        );
    }
}

/// Restore a shard's serialized state from DO storage.
pub fn restore_from_do_storage<T: serde::de::DeserializeOwned>(
    storage: &dyn DoStorage,
    shard_id: &str,
) -> Option<T> {
    let key = format!("shard:{shard_id}:state");
    storage
        .get_bytes(&key)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

// ---------------------------------------------------------------------------
// Subscriber entry — helper for DO fetch handlers
// ---------------------------------------------------------------------------

/// Registers a WebSocket-connected player with a shard running inside a DO.
///
/// The caller's DO fetch handler:
/// 1. Accepts the WebSocket upgrade.
/// 2. Gets a reference to the shard (held in DO-scoped state).
/// 3. Calls this function with the WS send closure and the resolved auth.
///
/// Returns a close handler — call it in the DO's webSocketClose event.
pub fn register_do_subscriber(
    shard: Arc<dyn DynShard>,
    subscriber_id: SubscriberId,
    ws_send: impl Fn(&[u8]) + Send + Sync + 'static,
    auth: ShardAuth,
) -> Result<DoSubscriberHandle, String> {
    let sink = do_websocket_sink(ws_send);
    shard
        .add_subscriber(subscriber_id.clone(), sink, &auth)
        .map_err(|e| e.to_string())?;
    Ok(DoSubscriberHandle {
        shard,
        subscriber_id,
    })
}

pub struct DoSubscriberHandle {
    shard: Arc<dyn DynShard>,
    subscriber_id: SubscriberId,
}

impl DoSubscriberHandle {
    /// Call on DO webSocketClose / webSocketError events.
    pub fn close(self) {
        self.shard.remove_subscriber(&self.subscriber_id);
    }
}

// ---------------------------------------------------------------------------
// Template: the JavaScript side of a Durable Object
// ---------------------------------------------------------------------------

/// The boilerplate a user adds to their Workers bundle's JS entry file.
///
/// This can't be generated from Rust alone (the DO class must be exported
/// from JS so the Workers runtime can instantiate it), so we ship this as
/// a string constant that the `pylon deploy --target workers` command
/// can drop into the generated bundle.
pub const DURABLE_OBJECT_TEMPLATE_JS: &str = r#"
// Auto-generated. One class per shard type.
// Wires up fetch handling, WebSocket accept/hibernation, and alarm-based ticks.
export class ShardDO {
  constructor(state, env) {
    this.state = state;
    this.env = env;
    this.sockets = new Map(); // sid -> WebSocket
    this.tickRateHz = env.TICK_RATE_HZ || 20;
  }

  async fetch(req) {
    const url = new URL(req.url);
    const sid = url.searchParams.get('sid') || 'anon';

    if (req.headers.get('Upgrade') === 'websocket') {
      const pair = new WebSocketPair();
      const [client, server] = Object.values(pair);
      this.state.acceptWebSocket(server); // hibernation-compatible
      this.sockets.set(sid, server);
      if (!(await this.state.storage.get('alarm_set'))) {
        await this.state.storage.setAlarm(Date.now() + (1000 / this.tickRateHz));
        await this.state.storage.put('alarm_set', true);
      }
      return new Response(null, { status: 101, webSocket: client });
    }
    return new Response('not found', { status: 404 });
  }

  async webSocketMessage(ws, message) {
    // Forward input JSON into the shard's input queue (via bound Wasm fn).
    this.env.SHARD_IMPORT.pushInput(this.state.id.toString(), message);
  }

  async webSocketClose(ws) {
    for (const [sid, s] of this.sockets) {
      if (s === ws) this.sockets.delete(sid);
    }
  }

  async alarm() {
    // Run one tick and broadcast to all connected sockets.
    const snapshot = this.env.SHARD_IMPORT.runTick(this.state.id.toString());
    for (const ws of this.sockets.values()) {
      try { ws.send(snapshot); } catch {}
    }
    // Reschedule.
    await this.state.storage.setAlarm(Date.now() + (1000 / this.tickRateHz));
  }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct InMemoryStorage {
        map: Mutex<std::collections::HashMap<String, Vec<u8>>>,
    }

    impl DoStorage for InMemoryStorage {
        fn get_bytes(&self, key: &str) -> Option<Vec<u8>> {
            self.map.lock().unwrap().get(key).cloned()
        }
        fn put_bytes(&self, key: &str, value: &[u8]) {
            self.map
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_vec());
        }
        fn delete(&self, key: &str) {
            self.map.lock().unwrap().remove(key);
        }
    }

    #[test]
    fn persist_and_restore_roundtrip() {
        let storage = InMemoryStorage {
            map: Mutex::new(std::collections::HashMap::new()),
        };

        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct State {
            score: u64,
            players: Vec<String>,
        }

        let original = State {
            score: 42,
            players: vec!["alice".into(), "bob".into()],
        };
        persist_to_do_storage(&storage, "match1", &original, 100);

        let restored: State = restore_from_do_storage(&storage, "match1").unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn do_websocket_sink_prepends_tick() {
        let captured = std::sync::Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
        let captured_clone = std::sync::Arc::clone(&captured);
        let sink = do_websocket_sink(move |bytes| {
            captured_clone.lock().unwrap().push(bytes.to_vec());
        });

        sink(42u64, b"hello");
        let all = captured.lock().unwrap();
        assert_eq!(all.len(), 1);
        // First 8 bytes: big-endian u64 tick number.
        assert_eq!(&all[0][..8], &42u64.to_be_bytes());
        assert_eq!(&all[0][8..], b"hello");
    }

    #[test]
    fn template_js_nonempty() {
        assert!(DURABLE_OBJECT_TEMPLATE_JS.contains("export class ShardDO"));
    }
}
