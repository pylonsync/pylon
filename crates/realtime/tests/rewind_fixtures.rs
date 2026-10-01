//! Lag compensation fixtures shared with the clients' interpolators
//! (packages/realtime/src/rewind.fixtures.json).
//!
//! This test drives the real replicator for one subscriber through moves
//! under a byte budget (so updates are deferred), component changes without
//! a move, quiet stretches, a despawn, and an id respawned. It records each
//! frame and, at many fractional render ticks, where the server's view of
//! that subscriber ([`pylon_replication::ViewHistory`]) places every entity.
//! The TypeScript and C# tests apply the same frames to their
//! `EntityInterpolator` and must place the same entities at the same
//! positions. The file must match what the server produces now; regenerate
//! it with `PYLON_WRITE_FIXTURES=1 cargo test -p pylon-realtime --test rewind_fixtures`.

use std::path::PathBuf;

use pylon_realtime::replication::{FrameInput, Plane, ReplicationConfig, Replicator};
use pylon_realtime::{InterestArea, Replicated};
use serde_json::{json, Value};

const HISTORY: u32 = 12;
const TICKS: u64 = 30;
const KEY: u64 = 1;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn fixtures() -> Value {
    let config = ReplicationConfig {
        precision: 0.05,
        // A few updates a tick: the far, quiet entities wait their turn.
        max_bytes_per_tick: 48,
        plane: Plane::XY,
    };
    let mut store = Replicated::new();
    for id in 1..=10u64 {
        store.spawn(id, [id as f32 * 4.0, 0.0, 0.0]);
        store.set_component(id, 1, &[100]);
    }
    let mut rep = Replicator::new();
    rep.set_history_ticks(HISTORY);
    let area = Some(InterestArea {
        x: 0.0,
        y: 0.0,
        radius: 30.0,
    });
    let mut frames = Vec::new();
    for tick in 1..=TICKS {
        let t = tick as f32;
        // 1 and 2 walk every tick; 3 moves every third tick (quiet between);
        // 4 changes only its hp; the rest drift slowly, far away.
        store.set_pos(1, [4.0 + t * 0.7, t * 0.3, 0.0]);
        store.set_pos(2, [8.0 - t * 0.45, 0.0, t * 0.1]);
        if tick % 3 == 0 {
            store.set_pos(3, [12.0 + t, -t, 0.0]);
        }
        if tick % 4 == 0 {
            store.set_component(4, 1, &[(100 - tick) as u8]);
        }
        for id in 5..=10u64 {
            store.set_pos(id, [id as f32 * 4.0 + t * 0.2, t * 0.05, 0.0]);
        }
        if tick == 12 {
            store.despawn(5);
        }
        if tick == 16 {
            store.spawn(5, [-3.0, 2.0, 0.0]);
        }
        rep.begin_tick(&store);
        let out = rep.frame(
            &store,
            &config,
            tick,
            FrameInput {
                key: KEY,
                visible: None,
                area,
                dropped: 0,
                queue_full: false,
                datagram: None,
            },
        );
        frames.push(json!({ "tick": tick, "frame": hex(&out.bytes) }));
    }

    // Render ticks the history covers, ascending (an interpolator never
    // goes back), at quarter ticks.
    let view = rep.view(KEY).expect("history is on");
    let mut queries = Vec::new();
    let mut r = (TICKS - HISTORY as u64) as f64;
    while r <= TICKS as f64 + 1.0 {
        let positions: Vec<Value> = view
            .ids()
            .filter_map(|id| {
                view.position_at(id, r)
                    .map(|p| json!([id, p[0], p[1], p[2]]))
            })
            .collect();
        queries.push(json!({ "tick": r, "positions": positions }));
        r += 0.25;
    }
    json!({ "frames": frames, "queries": queries })
}

#[test]
fn the_fixture_file_matches_the_server() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/realtime/src/rewind.fixtures.json");
    let now = fixtures();
    if std::env::var("PYLON_WRITE_FIXTURES").is_ok() {
        std::fs::write(&path, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        return;
    }
    // Compared as text: the file holds each f64 in its shortest exact form,
    // and serde_json's default parser does not read every one back exactly.
    let on_disk = std::fs::read_to_string(&path)
        .expect("no fixture file; run with PYLON_WRITE_FIXTURES=1 to write it");
    assert!(
        on_disk == serde_json::to_string_pretty(&now).unwrap() + "\n",
        "the rewind fixtures are out of date; run with PYLON_WRITE_FIXTURES=1"
    );
}

#[test]
fn the_fixtures_exercise_deferral_quiet_entities_and_respawns() {
    let f = fixtures();
    let frames = f["frames"].as_array().unwrap();
    assert_eq!(frames.len(), TICKS as usize);
    // The budget held some updates back: some frames are near the budget.
    assert!(frames
        .iter()
        .any(|fr| fr["frame"].as_str().unwrap().len() / 2 > 40));
    // Entity 5 is gone between its despawn and respawn, and back after.
    let at = |tick: f64| {
        f["queries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|q| q["tick"].as_f64() == Some(tick))
            .unwrap()["positions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p[0].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert!(at(29.0).contains(&5));
    assert!(at(29.0).contains(&1));
}
