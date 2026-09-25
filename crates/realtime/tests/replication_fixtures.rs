//! Replication frames shared with the TypeScript decoder
//! (packages/realtime/src/replication.fixtures.json).
//!
//! This test drives the real replicator through spawns, moves, component
//! changes and removals, despawns, id reuse, a precision change, and a
//! large id, and records each frame with the table the Rust decoder holds
//! after it. The TypeScript test applies the same frames and must reach the
//! same tables. The file must match what the encoder produces now: change
//! the format, and this test fails until the fixtures are regenerated with
//! `PYLON_WRITE_FIXTURES=1 cargo test -p pylon-realtime --test replication_fixtures`.

use std::path::PathBuf;

use pylon_realtime::replication::{FrameInput, Plane, ReplicationConfig, Replicator};
use pylon_realtime::{ReplicaTable, Replicated};
use serde_json::{json, Value};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn table_json(t: &ReplicaTable) -> Value {
    Value::Array(
        t.entities
            .iter()
            .map(|(id, e)| {
                let comps: serde_json::Map<String, Value> = e
                    .components
                    .iter()
                    .map(|(c, b)| (c.to_string(), Value::from(hex(b))))
                    .collect();
                json!({ "id": id, "q": e.q, "components": comps })
            })
            .collect(),
    )
}

fn fixtures() -> Value {
    let mut store = Replicated::new();
    let mut rep = Replicator::new();
    let mut table = ReplicaTable::new();
    let mut frames = Vec::new();
    let big: u64 = (1 << 53) - 1;
    let mut step =
        |store: &Replicated, rep: &mut Replicator, precision: f32, budget: usize, tick: u64| {
            let config = ReplicationConfig {
                precision,
                max_bytes_per_tick: budget,
                plane: Plane::XY,
            };
            rep.begin_tick(store);
            let out = rep.frame(
                store,
                &config,
                tick,
                FrameInput {
                    key: 1,
                    visible: None,
                    area: None,
                    dropped: 0,
                    queue_full: false,
                },
            );
            table.apply(&out.bytes).unwrap();
            frames.push(json!({
                "frame": hex(&out.bytes),
                "precision": precision,
                "table": table_json(&table),
            }));
        };

    store.spawn(1, [1.5, -2.25, 0.0]);
    store.spawn(40, [100.0, 0.0, 3.0]);
    store.spawn(big, [-7.0, 7.0, 0.0]);
    store.set_component(1, 1, b"hp");
    store.set_component(40, 9, &[]);
    step(&store, &mut rep, 0.01, 0, 1);

    store.set_pos(1, [1.75, -2.0, 0.0]);
    store.set_pos(big, [-7.5, 7.0, 0.0]);
    store.set_component(1, 1, b"hp2");
    store.set_component(1, 2, &[0, 255, 7]);
    step(&store, &mut rep, 0.01, 0, 2);

    store.remove_component(1, 1);
    store.despawn(40);
    store.spawn(41, [0.0, 0.0, -1.0]);
    step(&store, &mut rep, 0.01, 0, 3);

    // Id reuse: 41 goes away and comes back as a new entity in one tick.
    store.despawn(41);
    store.spawn(41, [9.0, 9.0, 9.0]);
    step(&store, &mut rep, 0.01, 0, 4);

    // A precision change forces a full frame.
    store.set_pos(1, [2.0, -2.0, 0.0]);
    step(&store, &mut rep, 0.5, 0, 5);

    // A small budget: some updates wait for later ticks.
    for id in 100..130u64 {
        store.spawn(id, [id as f32, 0.0, 0.0]);
    }
    step(&store, &mut rep, 0.5, 20, 6);
    for id in 100..130u64 {
        store.set_pos(id, [id as f32 + 3.0, 1.0, 0.0]);
    }
    for tick in 7..12 {
        step(&store, &mut rep, 0.5, 20, tick);
    }
    json!({ "frames": frames })
}

#[test]
fn the_fixture_file_matches_the_encoder() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/realtime/src/replication.fixtures.json");
    let now = fixtures();
    if std::env::var("PYLON_WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    let on_disk: Value = serde_json::from_str(
        &std::fs::read_to_string(&path)
            .expect("no fixture file; run with PYLON_WRITE_FIXTURES=1 to write it"),
    )
    .unwrap();
    assert_eq!(
        on_disk, now,
        "the replication fixtures are out of date; run with PYLON_WRITE_FIXTURES=1"
    );
}
