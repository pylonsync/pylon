//! WebAssembly shards: the arena guest from `crates/shard-guest/examples`
//! under the real tick code, plus module checks on hand-written modules.
//!
//! Needs the `wasm32-unknown-unknown` target
//! (`rustup target add wasm32-unknown-unknown`).

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use pylon_realtime::{
    DynShard, FrameKind, InputRejection, Shard, ShardAuth, ShardConfig, ShardError, ShardTicket,
    SnapshotFormat, SubscriberId,
};
use pylon_runtime::shard_wasm::{CreateError, WasmLimits, WasmShardHost, WasmShardKind, WasmSim};
use serde_json::{json, Value};

/// Build the guest examples once per test binary.
fn build_guests() -> &'static std::path::Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        // A target dir of its own, so this build never waits on the lock the
        // outer `cargo test` holds.
        let target_dir = root.join("target/shard-guest-tests");
        let status = Command::new(env!("CARGO"))
            .current_dir(&root)
            .args([
                "build",
                "-p",
                "pylon-shard-guest",
                "--examples",
                "--release",
                "--target",
                "wasm32-unknown-unknown",
                "--target-dir",
            ])
            .arg(&target_dir)
            .status()
            .expect("run cargo");
        assert!(
            status.success(),
            "building the guest examples failed; is wasm32-unknown-unknown installed?"
        );
        target_dir.join("wasm32-unknown-unknown/release/examples")
    })
}

fn guest_wasm(name: &str) -> Vec<u8> {
    std::fs::read(build_guests().join(format!("{name}.wasm"))).expect("read the guest module")
}

fn arena_wasm() -> &'static [u8] {
    static WASM: OnceLock<Vec<u8>> = OnceLock::new();
    WASM.get_or_init(|| guest_wasm("arena"))
}

fn config(format: SnapshotFormat) -> ShardConfig {
    ShardConfig {
        tick_rate_hz: 20,
        idle_ticks_before_shutdown: 0,
        snapshot_format: format,
        ..Default::default()
    }
}

fn kind(format: SnapshotFormat, limits: WasmLimits) -> WasmShardKind {
    WasmShardKind::compile("arena", arena_wasm(), config(format), limits).unwrap()
}

fn shard(format: SnapshotFormat, params: Value) -> Arc<Shard<WasmSim>> {
    let k = kind(format, WasmLimits::default());
    let sim = k.instantiate("a1", &params).unwrap();
    Shard::new("a1", sim, k.config().clone())
}

fn user(id: &str) -> ShardAuth {
    ShardAuth {
        user_id: Some(id.into()),
        ..Default::default()
    }
}

fn join(s: &Arc<Shard<WasmSim>>, id: &str) -> Arc<pylon_realtime::OutboundQueue> {
    DynShard::add_queued_subscriber(s.as_ref(), SubscriberId::new(id), &user(id)).unwrap()
}

fn send(s: &Arc<Shard<WasmSim>>, id: &str, body: Value) -> Result<u64, InputRejection> {
    DynShard::push_input_envelope(
        s.as_ref(),
        SubscriberId::new(id),
        SnapshotFormat::Json,
        body.to_string().as_bytes(),
        &user(id),
    )
}

/// The newest snapshot in the queue, decoded, and any rejections before it.
fn drain(q: &pylon_realtime::OutboundQueue, format: SnapshotFormat) -> (Value, Vec<Value>, u64) {
    let decode = |b: &[u8]| -> Value {
        match format {
            SnapshotFormat::MessagePack => rmp_serde::from_slice(b).unwrap(),
            _ => serde_json::from_slice(b).unwrap(),
        }
    };
    let mut snap = Value::Null;
    let mut ack = 0;
    let mut rejections = Vec::new();
    while let Some(f) = q.pop() {
        match f.kind {
            FrameKind::Snapshot => {
                snap = decode(&f.bytes);
                ack = f.ack;
            }
            FrameKind::InputRejected => rejections.push(decode(&f.bytes)),
            FrameKind::Replication => panic!("the arena guest sends snapshots"),
        }
    }
    (snap, rejections, ack)
}

#[test]
fn inputs_ticks_and_snapshots_in_json() {
    let s = shard(SnapshotFormat::Json, json!({ "label": "north" }));
    let q = join(&s, "u1");
    for seq in 1..=3 {
        send(
            &s,
            "u1",
            json!({ "input": { "move": { "dx": 1, "dy": 2 } }, "client_seq": seq }),
        )
        .unwrap();
    }
    s.run_tick();
    let (snap, rejections, ack) = drain(&q, SnapshotFormat::Json);
    assert!(rejections.is_empty(), "{rejections:?}");
    assert_eq!(ack, 3);
    assert_eq!(snap["label"], "north");
    assert_eq!(snap["players"], json!([{ "id": "u1", "x": 3, "y": 6 }]));
    // Fixed timestep: one tick at 20 Hz is 50 ms.
    assert_eq!(snap["elapsed_ms"], 50);
}

#[test]
fn msgpack_shard_takes_json_and_binary_inputs() {
    let s = shard(SnapshotFormat::MessagePack, json!({}));
    let q = join(&s, "u1");
    // A JSON text frame is re-encoded for the module.
    send(
        &s,
        "u1",
        json!({ "input": { "move": { "dx": 2, "dy": 0 } }, "client_seq": 1 }),
    )
    .unwrap();
    // A binary frame in the shard's codec passes through.
    let bin = rmp_serde::to_vec_named(
        &json!({ "input": { "move": { "dx": 0, "dy": 5 } }, "client_seq": 2 }),
    )
    .unwrap();
    DynShard::push_input_envelope(
        s.as_ref(),
        SubscriberId::new("u1"),
        SnapshotFormat::MessagePack,
        &bin,
        &user("u1"),
    )
    .unwrap();
    s.run_tick();
    let (snap, _, ack) = drain(&q, SnapshotFormat::MessagePack);
    assert_eq!(ack, 2);
    assert_eq!(snap["players"], json!([{ "id": "u1", "x": 2, "y": 5 }]));
}

#[test]
fn a_refused_input_comes_back_as_a_rejection() {
    let s = shard(SnapshotFormat::Json, json!({}));
    let q = join(&s, "u1");
    send(
        &s,
        "u1",
        json!({ "input": { "move": { "dx": 99, "dy": 0 } }, "client_seq": 7 }),
    )
    .unwrap();
    // An input the module cannot decode is refused before it is queued.
    let bad = send(
        &s,
        "u1",
        json!({ "input": { "teleport": {} }, "client_seq": 8 }),
    )
    .unwrap_err();
    assert_eq!(bad.code, "unauthorized");
    assert!(bad.message.contains("invalid input"), "{}", bad.message);
    s.run_tick();
    let (snap, rejections, ack) = drain(&q, SnapshotFormat::Json);
    assert_eq!(ack, 7);
    assert_eq!(rejections.len(), 1);
    assert_eq!(rejections[0]["code"], "apply_failed");
    assert_eq!(rejections[0]["client_seq"], 7);
    assert!(rejections[0]["message"]
        .as_str()
        .unwrap()
        .contains("too far"));
    assert_eq!(snap["players"], json!([]));
    assert!(s.is_running());
}

#[test]
fn snapshot_for_hides_other_players() {
    let s = shard(SnapshotFormat::Json, json!({ "fog": true }));
    let q1 = join(&s, "u1");
    let q2 = join(&s, "u2");
    send(
        &s,
        "u1",
        json!({ "input": { "move": { "dx": 1, "dy": 1 } } }),
    )
    .unwrap();
    send(
        &s,
        "u2",
        json!({ "input": { "move": { "dx": 2, "dy": 2 } } }),
    )
    .unwrap();
    s.run_tick();
    let (a, _, _) = drain(&q1, SnapshotFormat::Json);
    let (b, _, _) = drain(&q2, SnapshotFormat::Json);
    assert_eq!(a["players"], json!([{ "id": "u1", "x": 1, "y": 1 }]));
    assert_eq!(b["players"], json!([{ "id": "u2", "x": 2, "y": 2 }]));
}

#[test]
fn interest_management_runs_in_the_module() {
    let s = shard(SnapshotFormat::Json, json!({ "view_radius": 20.0 }));
    let names = ["u1", "u2", "ghost", "u3"];
    let queues: Vec<_> = names.iter().map(|n| join(&s, n)).collect();
    // A seer watches from u1's position and sees ghosts.
    let seer = DynShard::add_queued_subscriber(
        s.as_ref(),
        SubscriberId::new("seer-u1"),
        &ShardAuth::admin(),
    )
    .unwrap();
    let mv = |who: &str, dx: i64, dy: i64| {
        send(
            &s,
            who,
            json!({ "input": { "move": { "dx": dx, "dy": dy } } }),
        )
        .unwrap();
    };
    mv("u1", 1, 1);
    mv("u2", 2, 2);
    mv("ghost", 5, 5);
    for _ in 0..10 {
        mv("u3", 10, 0); // ends at (100, 0), out of everyone's range
    }
    // Ten inputs per subscriber fit the per-tick limit, so one tick applies
    // them all; the second tick snapshots the final positions.
    s.run_tick();
    s.run_tick();

    let ids = |q: &Arc<pylon_realtime::OutboundQueue>| -> (Vec<String>, Arc<[u8]>) {
        let mut last = None;
        while let Some(f) = q.pop() {
            if f.kind == FrameKind::Snapshot {
                last = Some(f.bytes);
            }
        }
        let bytes = last.expect("a snapshot");
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let mut names: Vec<String> = v["players"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect();
        names.sort();
        (names, bytes)
    };
    let (u1, b1) = ids(&queues[0]);
    let (u2, b2) = ids(&queues[1]);
    let (ghost, _) = ids(&queues[2]);
    let (u3, _) = ids(&queues[3]);
    let (seen_by_seer, _) = ids(&seer);
    // The ghost is in range of u1 and u2 but hidden from them.
    assert_eq!(u1, vec!["u1", "u2"]);
    assert_eq!(u2, vec!["u1", "u2"]);
    assert_eq!(seen_by_seer, vec!["ghost", "u1", "u2"]);
    assert_eq!(ghost, vec!["u1", "u2"]);
    assert_eq!(u3, vec!["u3"]);
    // u1 and u2 see the same players: one snapshot, one encoding.
    assert!(Arc::ptr_eq(&b1, &b2));
}

#[test]
fn a_module_with_part_of_the_interest_exports_is_refused() {
    let wasm = full_module(
        1,
        "i32.const 0",
        r#"(func (export "pylon_interest") (result i32) i32.const 0)"#,
    );
    let err = WasmShardKind::compile(
        "half",
        &wasm,
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .err()
    .unwrap();
    assert!(err.contains("but not all"), "{err}");
}

#[test]
fn authorization_hooks_run_in_the_module() {
    let s = shard(SnapshotFormat::Json, json!({}));
    // Default subscribe rule: the user id must be the subscriber id.
    let Err(err) =
        DynShard::add_queued_subscriber(s.as_ref(), SubscriberId::new("u2"), &user("u1"))
    else {
        panic!("u1 subscribed as u2");
    };
    assert!(
        matches!(err, ShardError::Unauthorized(ref m) if m.contains("does not match")),
        "{err:?}"
    );

    // A ticket admits a subscriber; the arena refuses inputs from a
    // spectator ticket.
    let spectator = ShardAuth {
        user_id: Some("u9".into()),
        ticket: Some(ShardTicket {
            shard: "a1".into(),
            sid: "watcher".into(),
            user_id: Some("u9".into()),
            exp: u64::MAX,
            claims: json!({ "role": "spectator" }),
        }),
        ..Default::default()
    };
    DynShard::add_queued_subscriber(s.as_ref(), SubscriberId::new("watcher"), &spectator).unwrap();
    let rejection = DynShard::push_input_envelope(
        s.as_ref(),
        SubscriberId::new("watcher"),
        SnapshotFormat::Json,
        br#"{"input":{"move":{"dx":1,"dy":1}},"client_seq":1}"#,
        &spectator,
    )
    .unwrap_err();
    assert_eq!(rejection.code, "unauthorized");
    assert!(rejection.message.contains("spectators"));
}

#[test]
fn a_panic_stops_the_shard_with_its_message() {
    let s = shard(SnapshotFormat::Json, json!({}));
    let q = join(&s, "u1");
    send(
        &s,
        "u1",
        json!({ "input": { "move": { "dx": 1, "dy": 1 } } }),
    )
    .unwrap();
    s.run_tick();
    drain(&q, SnapshotFormat::Json);
    send(&s, "u1", json!({ "input": "panic" })).unwrap();
    s.run_tick();
    assert!(!s.is_running());
    let failure = s.with_state(|sim| sim.failure()).unwrap();
    assert!(failure.contains("asked to panic"), "{failure}");
    // The last frame is null, never a stale snapshot that might hold what
    // a per-subscriber snapshot hid; then the connection closes.
    let (snap, _, _) = drain(&q, SnapshotFormat::Json);
    assert_eq!(snap, Value::Null);
    assert!(q.is_closed());
}

#[test]
fn a_tick_past_its_budget_stops_the_shard() {
    let k = kind(
        SnapshotFormat::Json,
        WasmLimits {
            budget: Duration::from_millis(50),
            ..Default::default()
        },
    );
    let s = Shard::new(
        "a1",
        k.instantiate("a1", &json!({})).unwrap(),
        k.config().clone(),
    );
    join(&s, "u1");
    send(&s, "u1", json!({ "input": "spin" })).unwrap();
    let start = Instant::now();
    s.run_tick();
    let took = start.elapsed();
    assert!(took < Duration::from_secs(2), "the spin ran for {took:?}");
    assert!(!s.is_running());
    let failure = s.with_state(|sim| sim.failure()).unwrap();
    assert!(failure.contains("time budget"), "{failure}");
}

#[test]
fn the_budget_covers_the_whole_tick_not_each_call() {
    let kind_with = |ms: u64| {
        kind(
            SnapshotFormat::Json,
            WasmLimits {
                budget: Duration::from_millis(ms),
                ..Default::default()
            },
        )
    };
    // Calibrate: iterations that take about 50 ms on this machine. The
    // fastest of three probes, so a noisy runner does not inflate it.
    let probe = kind_with(10_000);
    let s = Shard::new(
        "p",
        probe.instantiate("p", &json!({})).unwrap(),
        probe.config().clone(),
    );
    join(&s, "u1");
    let probe_iters: u64 = 20_000_000;
    let mut fastest = Duration::MAX;
    for _ in 0..3 {
        send(
            &s,
            "u1",
            json!({ "input": { "burn": { "iters": probe_iters } } }),
        )
        .unwrap();
        let start = Instant::now();
        s.run_tick();
        fastest = fastest.min(start.elapsed());
    }
    let per_iter = fastest.as_secs_f64() / probe_iters as f64;
    let iters = (0.050 / per_iter) as u64;

    // 32 inputs of ~50 ms in one tick (~1.6 s) against a 400 ms budget:
    // each call fits, the tick does not, with a 4x margin for a slow runner.
    let k = kind_with(400);
    let s = Shard::new(
        "a1",
        k.instantiate("a1", &json!({})).unwrap(),
        k.config().clone(),
    );
    join(&s, "u1");
    for _ in 0..32 {
        send(&s, "u1", json!({ "input": { "burn": { "iters": iters } } })).unwrap();
    }
    s.run_tick();
    assert!(!s.is_running());
    let failure = s.with_state(|sim| sim.failure()).unwrap();
    assert!(failure.contains("time budget"), "{failure}");

    // One such input per tick runs fine, tick after tick (8x margin).
    let s = Shard::new(
        "a2",
        k.instantiate("a2", &json!({})).unwrap(),
        k.config().clone(),
    );
    join(&s, "u1");
    for _ in 0..8 {
        send(&s, "u1", json!({ "input": { "burn": { "iters": iters } } })).unwrap();
        s.run_tick();
    }
    assert!(s.is_running(), "{:?}", s.with_state(|sim| sim.failure()));
}

#[test]
fn memory_past_the_cap_stops_the_shard() {
    let k = kind(
        SnapshotFormat::Json,
        WasmLimits {
            memory_bytes: 16 << 20,
            ..Default::default()
        },
    );
    let s = Shard::new(
        "a1",
        k.instantiate("a1", &json!({})).unwrap(),
        k.config().clone(),
    );
    join(&s, "u1");
    send(&s, "u1", json!({ "input": "grow" })).unwrap();
    s.run_tick();
    assert!(!s.is_running());
    let failure = s.with_state(|sim| sim.failure()).unwrap();
    assert!(
        failure.contains("trapped") && failure.contains("memory"),
        "{failure}"
    );
}

#[test]
fn a_trap_in_an_authorize_hook_stops_an_idle_event_driven_shard() {
    let k = WasmShardKind::compile(
        "arena",
        arena_wasm(),
        ShardConfig {
            tick_rate_hz: 0,
            idle_ticks_before_shutdown: 0,
            snapshot_format: SnapshotFormat::Json,
            ..Default::default()
        },
        WasmLimits::default(),
    )
    .unwrap();
    let s = Shard::new(
        "ev",
        k.instantiate("ev", &json!({})).unwrap(),
        k.config().clone(),
    );
    let q = join(&s, "u1");
    let rejection = send(&s, "u1", json!({ "input": "trap_auth" })).unwrap_err();
    assert_eq!(rejection.code, "unauthorized");
    // No tick ran, and none would: the shard stops at once and its
    // subscribers' queues close so their transports disconnect.
    assert!(!s.is_running());
    assert!(q.is_closed());
    assert!(s
        .with_state(|sim| sim.failure())
        .unwrap()
        .contains("authorize_input"));
}

#[test]
fn a_failed_subscriber_snapshot_never_falls_back_to_the_broadcast() {
    // Fog: each subscriber sees only itself. The broadcast (all players) is
    // cached once; then the module panics.
    let s = shard(SnapshotFormat::Json, json!({ "fog": true }));
    let q1 = join(&s, "u1");
    join(&s, "u2");
    send(
        &s,
        "u1",
        json!({ "input": { "move": { "dx": 1, "dy": 1 } } }),
    )
    .unwrap();
    send(
        &s,
        "u2",
        json!({ "input": { "move": { "dx": 2, "dy": 2 } } }),
    )
    .unwrap();
    s.run_tick();
    let all = s.snapshot(); // the broadcast, with both players
    assert!(String::from_utf8_lossy(all.bytes()).contains("u2"));
    while q1.pop().is_some() {}
    send(&s, "u1", json!({ "input": "panic" })).unwrap();
    s.run_tick();
    let mut frames = Vec::new();
    while let Some(f) = q1.pop() {
        frames.push(String::from_utf8_lossy(&f.bytes).into_owned());
    }
    assert!(frames.iter().all(|f| !f.contains("u2")), "{frames:?}");
}

#[test]
fn a_finished_module_stops_the_shard() {
    let s = shard(SnapshotFormat::Json, json!({}));
    join(&s, "u1");
    send(&s, "u1", json!({ "input": "finish" })).unwrap();
    s.run_tick();
    assert!(!s.is_running());
    assert_eq!(s.with_state(|sim| sim.failure()), None);
}

#[test]
fn identical_inputs_give_identical_snapshots() {
    let run = || {
        let s = shard(SnapshotFormat::MessagePack, json!({}));
        let q = join(&s, "u1");
        for i in 0..50 {
            send(
                &s,
                "u1",
                json!({ "input": { "move": { "dx": i % 7 - 3, "dy": i % 5 - 2 } } }),
            )
            .unwrap();
            s.run_tick();
        }
        let mut last = None;
        while let Some(f) = q.pop() {
            last = Some(f.bytes);
        }
        last.unwrap()
    };
    assert_eq!(run(), run());
}

#[test]
fn host_creates_limits_and_stops_shards() {
    let host = WasmShardHost::new(vec![kind(
        SnapshotFormat::Json,
        WasmLimits {
            max_instances: 2,
            ..Default::default()
        },
    )]);
    let info = host.create("arena", "m1", &json!({})).unwrap();
    assert_eq!((info.kind.as_str(), info.running), ("arena", true));
    assert_eq!(
        host.create("arena", "m1", &json!({})).unwrap_err(),
        CreateError::Exists("m1".into())
    );
    host.create("arena", "m2", &json!({})).unwrap();
    assert!(matches!(
        host.create("arena", "m3", &json!({})),
        Err(CreateError::LimitReached { max: 2, .. })
    ));
    assert!(matches!(
        host.create("nope", "m3", &json!({})),
        Err(CreateError::UnknownKind(_))
    ));
    assert!(matches!(
        host.create("arena", "bad id", &json!({})),
        Err(CreateError::InvalidId(_))
    ));
    // The module's init can refuse its params.
    host.stop("m2");
    match host.create("arena", "m3", &json!({ "label": "" })) {
        Err(CreateError::Init(why)) => assert!(why.contains("label must not be empty"), "{why}"),
        other => panic!("expected an init failure, got {other:?}"),
    }
    // Stopping frees a slot, and the tick loop runs the shard.
    host.create("arena", "m3", &json!({})).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let m1 = host.info("m1").unwrap();
    assert!(m1.tick > 0, "{m1:?}");
    assert!(host.stop("m1"));
    assert!(host.info("m1").is_none());
    assert_eq!(
        host.list()
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        vec!["m3"]
    );
    host.stop_all();
}

fn wat_module(body: &str) -> Vec<u8> {
    wat::parse_str(body).unwrap()
}

#[test]
fn modules_with_other_imports_or_missing_exports_are_refused() {
    let wasi = wat_module(
        r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))
                   (memory (export "memory") 1))"#,
    );
    let err = WasmShardKind::compile(
        "w",
        &wasi,
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .err()
    .unwrap();
    assert!(err.contains("may import only pylon.log"), "{err}");

    let empty = wat_module(r#"(module (memory (export "memory") 1))"#);
    let err = WasmShardKind::compile(
        "e",
        &empty,
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .err()
    .unwrap();
    assert!(
        err.contains("missing exports") && err.contains("pylon_tick"),
        "{err}"
    );

    let err = WasmShardKind::compile(
        "b",
        b"not wasm",
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .err()
    .unwrap();
    assert!(err.contains("invalid WebAssembly"), "{err}");

    let err = WasmShardKind::compile(
        "c",
        arena_wasm(),
        config(SnapshotFormat::Bincode),
        WasmLimits::default(),
    )
    .err()
    .unwrap();
    assert!(err.contains("bincode"), "{err}");
}

/// A module with every export. `abi` is what pylon_shard_abi returns;
/// `apply_input` is the body of pylon_apply_input (returns i32); `extra` is
/// added at module level.
fn full_module(abi: i32, apply_input: &str, extra: &str) -> Vec<u8> {
    let mut exports = String::new();
    for (name, sig) in [
        ("pylon_shard_abi", format!("(result i32) i32.const {abi}")),
        (
            "pylon_scratch",
            "(param i32) (result i32) i32.const 1024".into(),
        ),
        ("pylon_output_ptr", "(result i32) i32.const 0".into()),
        ("pylon_output_len", "(result i32) i32.const 0".into()),
        (
            "pylon_init",
            "(param i32 i32 i32) (result i32) i32.const 0".into(),
        ),
        (
            "pylon_apply_input",
            format!("(param i32 i32 i32 i32) (result i32) {apply_input}"),
        ),
        ("pylon_tick", "(param i64)".into()),
        ("pylon_snapshot", "(result i32) i32.const 0".into()),
        (
            "pylon_snapshot_for",
            "(param i32 i32) (result i32) i32.const 2".into(),
        ),
        ("pylon_is_finished", "(result i32) i32.const 0".into()),
        (
            "pylon_authorize_subscribe",
            "(param i32 i32 i32 i32) (result i32) i32.const 0".into(),
        ),
        (
            "pylon_authorize_input",
            "(param i32 i32 i32 i32 i32 i32) (result i32) i32.const 0".into(),
        ),
    ] {
        exports.push_str(&format!("(func (export \"{name}\") {sig})\n"));
    }
    wat_module(&format!(
        "(module (memory (export \"memory\") 1) {extra} {exports})"
    ))
}

#[test]
fn a_module_that_claims_another_abi_is_refused() {
    let wasm = full_module(2, "i32.const 0", "");
    let k = WasmShardKind::compile(
        "v2",
        &wasm,
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .unwrap();
    let err = k.instantiate("x", &json!({})).err().unwrap();
    assert!(err.contains("shard ABI 2"), "{err}");
}

#[test]
fn deep_recursion_on_a_small_stack_stops_the_shard_not_the_process() {
    // Recursion on the native wasm stack, not a shadow stack in memory.
    let wasm = full_module(
        1,
        "i32.const 0 call $down drop i32.const 0",
        "(func $down (param i32) (result i32) local.get 0 i32.const 1 i32.add call $down)",
    );
    let k = WasmShardKind::compile(
        "deep",
        &wasm,
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .unwrap();
    let s = Shard::new(
        "d",
        k.instantiate("d", &json!({})).unwrap(),
        k.config().clone(),
    );
    join(&s, "u1");
    send(&s, "u1", json!({ "input": 1 })).unwrap();
    // HTTP workers that run authorize hooks have 512 KiB stacks.
    let s2 = Arc::clone(&s);
    std::thread::Builder::new()
        .stack_size(512 << 10)
        .spawn(move || s2.run_tick())
        .unwrap()
        .join()
        .unwrap();
    assert!(!s.is_running());
    let failure = s.with_state(|sim| sim.failure()).unwrap();
    assert!(failure.contains("call stack exhausted"), "{failure}");
}

// ---------------------------------------------------------------------------
// Entity replication in a WebAssembly shard (the `field` guest)
// ---------------------------------------------------------------------------

fn field(params: Value) -> Arc<Shard<WasmSim>> {
    let k = WasmShardKind::compile(
        "field",
        &guest_wasm("field"),
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .unwrap();
    let sim = k.instantiate("f1", &params).unwrap();
    Shard::new("f1", sim, k.config().clone())
}

fn apply_all(q: &pylon_realtime::OutboundQueue, table: &mut pylon_realtime::ReplicaTable) -> bool {
    let mut full = false;
    while let Some(f) = q.pop() {
        if f.kind == FrameKind::Replication {
            full |= table.apply(&f.bytes).unwrap().full;
        }
    }
    full
}

#[test]
fn a_wasm_shard_replicates_its_store_with_interest_and_stealth() {
    let s = field(json!({ "units": 40, "radius": 12.0 }));
    assert!(DynShard::replicates(s.as_ref()));
    let plain = join(&s, "u5");
    let seer = DynShard::add_queued_subscriber(
        s.as_ref(),
        SubscriberId::new("seer1"),
        &ShardAuth::admin(),
    )
    .unwrap();
    let (mut tp, mut ts) = (
        pylon_realtime::ReplicaTable::new(),
        pylon_realtime::ReplicaTable::new(),
    );
    let mut seer_saw_zero = false;
    for tick in 0..40 {
        if tick == 10 {
            send(&s, "u5", json!({ "input": { "hp": [6, 42] } })).unwrap();
        }
        if tick == 20 {
            send(&s, "u5", json!({ "input": { "despawn": 7 } })).unwrap();
        }
        s.run_tick();
        assert!(!apply_all(&plain, &mut tp) || tick == 0);
        apply_all(&seer, &mut ts);
        // The stealthed unit never reaches the plain subscriber.
        assert!(!tp.entities.contains_key(&0));
        seer_saw_zero |= ts.entities.contains_key(&0);
        // Far units are out of the plain subscriber's view.
        assert!(!tp.entities.contains_key(&39));
    }
    assert!(seer_saw_zero);
    // The component change and the despawn arrived; positions track the
    // module's (unit 5 is at the center of its own view).
    assert_eq!(tp.entities[&6].components[&1], vec![42]);
    assert!(!tp.entities.contains_key(&7));
    assert!(tp.pos(5).is_some());
}

#[test]
fn a_wasm_replicating_shard_sends_a_baseline_to_a_reconnect() {
    let s = field(json!({ "units": 5 }));
    let q = join(&s, "u1");
    let mut t = pylon_realtime::ReplicaTable::new();
    s.run_tick();
    assert!(apply_all(&q, &mut t));
    assert_eq!(t.entities.len(), 4); // unit 0 is stealthed
    s.run_tick();
    assert!(!apply_all(&q, &mut t));
    assert!(s.remove_queued_subscriber(&q));
    let again = join(&s, "u1");
    let mut fresh = pylon_realtime::ReplicaTable::new();
    s.run_tick();
    assert!(apply_all(&again, &mut fresh));
    assert_eq!(fresh.entities.len(), 4);
}

#[test]
fn a_guest_that_swaps_in_a_new_store_resends_everything() {
    let s = field(json!({ "units": 5 }));
    let q = join(&s, "u1");
    let mut t = pylon_realtime::ReplicaTable::new();
    s.run_tick();
    apply_all(&q, &mut t);
    assert_eq!(t.entities.len(), 4);
    // Ids 0..3 come back in a new store, at new positions.
    send(&s, "u1", json!({ "input": { "reset": 3 } })).unwrap();
    s.run_tick();
    s.run_tick();
    apply_all(&q, &mut t);
    let ids: Vec<u64> = t.entities.keys().copied().collect();
    assert_eq!(ids, vec![1, 2]); // unit 0 stealthed; 3 and 4 gone
    for id in [1u64, 2] {
        let x = t.pos(id).unwrap()[0];
        // Units walk a circle of radius 2 around their center.
        assert!(
            (x - (1000.0 + id as f32 * 3.0)).abs() <= 2.01,
            "unit {id} at {x}"
        );
        assert_eq!(t.entities[&id].components[&1], vec![50]);
    }
}

#[test]
fn a_replicated_store_past_its_limit_stops_the_shard() {
    let k = WasmShardKind::compile(
        "field",
        &guest_wasm("field"),
        config(SnapshotFormat::Json),
        WasmLimits {
            max_replicated_entities: 20,
            ..Default::default()
        },
    )
    .unwrap();
    let s = Shard::new(
        "f1",
        k.instantiate("f1", &json!({ "units": 5 })).unwrap(),
        k.config().clone(),
    );
    join(&s, "u1");
    s.run_tick();
    assert!(s.is_running());
    send(&s, "u1", json!({ "input": { "flood": 50 } })).unwrap();
    s.run_tick();
    assert!(!s.is_running());
    let failure = s.with_state(|sim| sim.failure()).unwrap();
    assert!(failure.contains("past its limit"), "{failure}");
}

#[test]
fn the_host_registry_names_kinds_reports_numbers_and_stops_through_the_host() {
    use pylon_realtime::DynShardRegistry;
    let host = WasmShardHost::new(vec![kind(SnapshotFormat::Json, WasmLimits::default())]);
    host.create("arena", "m1", &json!({})).unwrap();
    let registry: &dyn DynShardRegistry = host.as_ref();
    assert_eq!(registry.kind("m1").as_deref(), Some("arena"));
    assert_eq!(registry.kind("nope"), None);
    assert_eq!(registry.failure("m1"), None);
    assert!(registry.get("m1").unwrap().is_running());
    // Stopping through the registry removes the host's bookkeeping too: the
    // kind's slot frees and the id can be created again.
    assert!(registry.stop("m1"));
    assert!(host.info("m1").is_none());
    assert!(registry.get("m1").is_none());
    assert!(!registry.stop("m1"));
    host.create("arena", "m1", &json!({})).unwrap();
}

#[test]
fn a_saved_shard_starts_again_where_it_was() {
    let k = kind(SnapshotFormat::Json, WasmLimits::default());
    assert!(k.saves_state());
    let params = json!({ "label": "north" });
    let s = Shard::new(
        "a1",
        k.instantiate("a1", &params).unwrap(),
        k.config().clone(),
    );
    let _q = join(&s, "u1");
    send(
        &s,
        "u1",
        json!({ "input": { "move": { "dx": 3, "dy": 4 } } }),
    )
    .unwrap();
    s.run_tick();
    let saved = s
        .with_state(|sim| sim.save())
        .unwrap()
        .expect("the arena saves");

    // Another instance, on another machine, from the saved bytes.
    let t = Shard::new(
        "a1",
        k.restore("a1", &params, &saved).unwrap(),
        k.config().clone(),
    );
    let q = join(&t, "u2");
    t.run_tick();
    let (snap, _, _) = drain(&q, SnapshotFormat::Json);
    assert_eq!(snap["label"], "north");
    assert_eq!(snap["players"], json!([{ "id": "u1", "x": 3, "y": 4 }]));

    // Bad bytes are the module's refusal, not a crash.
    let err = k.restore("a1", &params, b"not json").err().unwrap();
    assert!(err.contains("pylon_restore refused"), "{err}");
}

#[test]
fn a_module_without_saved_state_saves_nothing_and_refuses_restore() {
    let k = WasmShardKind::compile(
        "field",
        &guest_wasm("field"),
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .unwrap();
    // The SDK exports the pair; the field guest keeps the default (none).
    assert!(k.saves_state());
    let sim = k.instantiate("f1", &json!({ "units": 3 })).unwrap();
    assert_eq!(sim.save().unwrap(), None);

    // A module that exports only half of the pair is refused.
    let half = full_module(
        1,
        "i32.const 0",
        r#"(func (export "pylon_save") (result i32) i32.const 3)"#,
    );
    let err = WasmShardKind::compile(
        "half",
        &half,
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .err()
    .unwrap();
    assert!(
        err.contains("pylon_save") && err.contains("pylon_restore"),
        "{err}"
    );

    // A module without the pair (built before saved state) saves nothing
    // and cannot restore.
    let old = WasmShardKind::compile(
        "old",
        &full_module(1, "i32.const 0", ""),
        config(SnapshotFormat::Json),
        WasmLimits::default(),
    )
    .unwrap();
    assert!(!old.saves_state());
    assert_eq!(
        old.instantiate("o1", &json!({})).unwrap().save().unwrap(),
        None
    );
    assert!(old.restore("o1", &json!({}), b"{}").is_err());
}
