//! A shard WebSocket server with one MessagePack shard, for the client
//! end-to-end tests (tools/smoke-shard-codecs.sh).
//!
//! Prints one JSON line when ready:
//! `{"port": 1234, "shard": "arena", "tokens": {"ts": "...", "swift": "..."}}`
//! and serves until killed.
//!
//! The shard moves each subscriber's player by the `{dx, dy}` inputs it
//! sends. An input with `dx == -999` fails in `apply_input`, so clients can
//! test rejection frames.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_auth::SessionStore;
use pylon_realtime::{
    DynShardRegistry, Shard, ShardConfig, ShardRegistry, SimState, SnapshotFormat, SubscriberId,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Move {
    dx: i32,
    dy: i32,
}

#[derive(Serialize, Clone)]
struct Player {
    id: String,
    x: i32,
    y: i32,
}

#[derive(Serialize, Clone)]
struct ArenaSnapshot {
    players: Vec<Player>,
    label: String,
}

#[derive(Default)]
struct Arena {
    players: BTreeMap<String, (i32, i32)>,
}

impl SimState for Arena {
    type Input = Move;
    type Snapshot = ArenaSnapshot;
    type Error = String;

    fn apply_input(
        &mut self,
        sub: &SubscriberId,
        input: Move,
        _now: Instant,
    ) -> Result<(), String> {
        if input.dx == -999 {
            return Err("dx -999 is not a move".into());
        }
        let p = self.players.entry(sub.as_str().to_string()).or_default();
        p.0 += input.dx;
        p.1 += input.dy;
        Ok(())
    }

    fn tick(&mut self, _dt: Duration) {}

    fn snapshot(&self) -> ArenaSnapshot {
        ArenaSnapshot {
            players: self
                .players
                .iter()
                .map(|(id, (x, y))| Player {
                    id: id.clone(),
                    x: *x,
                    y: *y,
                })
                .collect(),
            label: "arena".into(),
        }
    }
}

fn main() {
    let registry: Arc<ShardRegistry<Arena>> = Arc::new(ShardRegistry::new());
    registry.insert(Shard::new(
        "arena",
        Arena::default(),
        ShardConfig {
            tick_rate_hz: 20,
            idle_ticks_before_shutdown: 0,
            snapshot_format: SnapshotFormat::MessagePack,
            ..Default::default()
        },
    ));
    let sessions = Arc::new(SessionStore::new());
    let ts = sessions.create("ts".into()).token;
    let swift = sessions.create("swift".into()).token;

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    println!(
        "{}",
        serde_json::json!({
            "port": port,
            "shard": "arena",
            "tokens": { "ts": ts, "swift": swift },
        })
    );
    let dyn_registry: Arc<dyn DynShardRegistry> = registry;
    pylon_runtime::shard_ws::serve(listener, dyn_registry, sessions, 0);
}
