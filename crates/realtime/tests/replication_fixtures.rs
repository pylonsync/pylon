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

use pylon_realtime::replication::{
    DatagramInput, FrameInput, FrameOutput, Plane, ReplicationConfig, Replicator,
};
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
                    datagram: None,
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
    json!({ "frames": frames, "datagrams": datagram_fixtures() })
}

/// Datagram-mode output (see `pylon_replication::datagram`), applied in an
/// order a lossy network could produce: a reversed pair (the older one is
/// skipped), a late datagram for an id that spawned again since (skipped
/// by its generation), and one tick split across small datagrams. Each
/// event records what the Rust table did.
fn datagram_fixtures() -> Value {
    let mut store = Replicated::new();
    let mut rep = Replicator::new();
    let mut table = ReplicaTable::new();
    let mut events: Vec<Value> = Vec::new();
    let build =
        |store: &Replicated, rep: &mut Replicator, tick: u64, max_size: usize| -> FrameOutput {
            rep.begin_tick(store);
            rep.frame(
                store,
                &ReplicationConfig {
                    precision: 0.01,
                    max_bytes_per_tick: 0,
                    plane: Plane::XY,
                },
                tick,
                FrameInput {
                    key: 1,
                    visible: None,
                    area: None,
                    dropped: 0,
                    queue_full: false,
                    datagram: Some(DatagramInput {
                        max_size,
                        input_ack: tick * 10,
                    }),
                },
            )
        };
    let stream = |table: &mut ReplicaTable, events: &mut Vec<Value>, bytes: &[u8]| {
        table.apply(bytes).unwrap();
        events.push(json!({ "stream": hex(bytes), "table": table_json(table) }));
    };
    let datagram = |table: &mut ReplicaTable, events: &mut Vec<Value>, bytes: &[u8]| {
        let s = table.apply_datagram(bytes).unwrap();
        events.push(json!({
            "datagram": hex(bytes),
            "frame": s.frame,
            "tick": s.tick,
            "ack": s.ack,
            "updated": s.updated,
            "skipped": s.skipped,
            "framesApplied": table.frames_applied,
            "table": table_json(table),
        }));
    };

    store.spawn(1, [1.0, 1.0, 0.0]);
    store.spawn(2, [2.0, 2.0, 0.0]);
    store.set_component(2, 5, b"hp");
    store.spawn(3, [3.0, 3.0, 0.0]);
    let out = build(&store, &mut rep, 1, 1200);
    stream(&mut table, &mut events, &out.bytes);

    // Two ticks of moves; the second tick's datagram arrives first.
    store.set_pos(1, [1.5, 1.0, 0.0]);
    store.set_pos(2, [2.0, 2.5, 0.0]);
    store.set_component(2, 5, b"hp2");
    let first = build(&store, &mut rep, 2, 1200).datagrams;
    store.set_pos(1, [1.75, 1.0, 0.0]);
    store.remove_component(2, 5);
    let second = build(&store, &mut rep, 3, 1200).datagrams;
    for d in second.iter().chain(&first) {
        datagram(&mut table, &mut events, d);
    }

    // Entity 3 moves; that datagram is late. Meanwhile 3 is despawned and
    // spawned again under the same id, so the late one is for the old
    // generation.
    store.set_pos(3, [3.5, 3.0, 0.0]);
    let late = build(&store, &mut rep, 4, 1200).datagrams;
    store.despawn(3);
    store.spawn(3, [30.0, 30.0, 0.0]);
    let out = build(&store, &mut rep, 5, 1200);
    stream(&mut table, &mut events, &out.bytes);
    for d in &late {
        datagram(&mut table, &mut events, d);
    }

    // Many entities move in one tick; small datagrams split them.
    for id in 10..40u64 {
        store.spawn(id, [id as f32, 0.0, 0.0]);
    }
    let out = build(&store, &mut rep, 6, 1200);
    stream(&mut table, &mut events, &out.bytes);
    for id in 10..40u64 {
        store.set_pos(id, [id as f32, 2.0, 0.0]);
    }
    let out = build(&store, &mut rep, 7, 60);
    assert!(out.datagrams.len() > 2, "the tick splits");
    for d in &out.datagrams {
        datagram(&mut table, &mut events, d);
    }
    Value::Array(events)
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
