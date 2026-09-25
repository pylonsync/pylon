//! A shard WebSocket server with one replicating shard, for the client
//! end-to-end tests (tools/smoke-shard-codecs.sh).
//!
//! Prints one JSON line when ready:
//! `{"port": 1234, "shard": "field", "tokens": {"ts": "...", "swift": "..."}}`
//! and serves until killed.
//!
//! Ten units walk in circles. Unit 0 is stealthed: a subscriber whose id
//! starts with "seer" sees it, others never do. Input `[id, hp]` sets a
//! unit's hp component (id 1); input `[id, 0]` despawns the unit.

use std::net::TcpListener;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_auth::SessionStore;
use pylon_realtime::{
    DynShardRegistry, EntityId, Plane, Replicated, ReplicatedRef, ReplicationConfig, Shard,
    ShardConfig, ShardRegistry, SimState, SubscriberId,
};

const HP: u8 = 1;

struct Field {
    store: Replicated,
    t: f32,
}

impl SimState for Field {
    type Input = (EntityId, u8);
    type Snapshot = ();
    type Error = String;

    fn apply_input(
        &mut self,
        _: &SubscriberId,
        (id, hp): (EntityId, u8),
        _: Instant,
    ) -> Result<(), String> {
        if hp == 0 {
            self.store.despawn(id);
            return Ok(());
        }
        if self.store.set_component(id, HP, &[hp]) {
            Ok(())
        } else {
            Err(format!("no unit {id}"))
        }
    }

    fn tick(&mut self, dt: Duration) {
        self.t += dt.as_secs_f32();
        let ids: Vec<EntityId> = self.store.iter().map(|(id, _)| id).collect();
        for id in ids {
            let a = self.t + id as f32;
            self.store
                .set_pos(id, [id as f32 * 5.0 + a.cos(), a.sin(), 0.0]);
        }
    }

    fn snapshot(&self) {}

    fn can_see(&self, sid: &SubscriberId, entity: EntityId) -> bool {
        entity != 0 || sid.as_str().starts_with("seer")
    }

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

fn main() {
    let mut store = Replicated::new();
    for id in 0..10u64 {
        store.spawn(id, [id as f32 * 5.0, 0.0, 0.0]);
        store.set_component(id, HP, &[100]);
    }
    let registry: Arc<ShardRegistry<Field>> = Arc::new(ShardRegistry::new());
    registry.insert(Shard::new(
        "field",
        Field { store, t: 0.0 },
        ShardConfig {
            tick_rate_hz: 20,
            idle_ticks_before_shutdown: 0,
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
        serde_json::json!({ "port": port, "shard": "field", "tokens": { "ts": ts, "swift": swift } })
    );
    let dyn_registry: Arc<dyn DynShardRegistry> = registry;
    pylon_runtime::shard_ws::serve(listener, dyn_registry, sessions, 0);
}
