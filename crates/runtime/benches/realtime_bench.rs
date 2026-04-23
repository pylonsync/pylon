//! Realtime-path microbenchmarks.
//!
//! These feed the SIZING.md table. Run with:
//!
//! ```sh
//! cargo bench -p pylon-runtime --bench realtime_bench
//! ```
//!
//! Three axes:
//! 1. `change_log append` — the hottest sync path; every write produces one.
//! 2. `change_log pull cycle` — what a polling client pays.
//! 3. `ws broadcast fanout` — how many messages the WS hub can fan out
//!    per second. No real sockets: we measure the enqueue path since that
//!    is the work a writer thread actually does. The shard worker threads
//!    drain in the background.

use std::sync::Arc;
use std::time::Instant;

use pylon_runtime::ws::WsHub;
use pylon_sync::{ChangeEvent, ChangeKind, ChangeLog, SyncCursor};

fn bench(name: &str, iterations: u32, mut f: impl FnMut()) {
    for _ in 0..10 {
        f();
    }
    let start = Instant::now();
    for _ in 0..iterations {
        f();
    }
    let elapsed = start.elapsed();
    let per_op = elapsed / iterations;
    let ops_sec = if per_op.as_nanos() > 0 {
        1_000_000_000 / per_op.as_nanos()
    } else {
        0
    };
    println!(
        "  {:<36} {:>8} ops  {:>10.2?} total  {:>8.2?}/op  {:>8} ops/sec",
        name, iterations, elapsed, per_op, ops_sec
    );
}

fn sample_event(i: u64) -> ChangeEvent {
    ChangeEvent {
        seq: 0,
        entity: "Todo".into(),
        row_id: format!("row_{i}"),
        kind: ChangeKind::Insert,
        data: Some(serde_json::json!({"title": format!("todo {i}"), "done": false})),
        timestamp: String::new(),
    }
}

fn main() {
    println!("\npylon realtime benchmarks\n");

    // -- ChangeLog append --
    let log = ChangeLog::new();
    let mut i = 0u64;
    bench("change_log.append", 100_000, || {
        i += 1;
        log.append("Todo", &format!("row_{i}"), ChangeKind::Insert, None);
    });

    // -- ChangeLog pull cycle (1000 events, cursor advancing) --
    let log = ChangeLog::new();
    for j in 0..10_000u64 {
        log.append("Todo", &format!("row_{j}"), ChangeKind::Insert, None);
    }
    let mut cursor_seq: u64 = 0;
    bench("change_log.pull(100)", 10_000, || {
        let _ = log
            .pull(
                &SyncCursor {
                    last_seq: cursor_seq,
                },
                100,
            )
            .unwrap();
        cursor_seq = (cursor_seq + 100) % 9_900;
    });

    // -- WS broadcast enqueue throughput --
    // Measures the work a writer thread does: serialize event + try_send to
    // each shard channel. Shard worker threads drain in the background, but
    // with zero clients attached they no-op, so the rate you see here is
    // the enqueue-side ceiling. Real throughput depends on client count
    // and message size.
    let hub = WsHub::new();
    let _hub_clone: Arc<WsHub> = Arc::clone(&hub);
    let mut i = 0u64;
    bench("ws_hub.broadcast (enqueue, 0 clients)", 100_000, || {
        i += 1;
        hub.broadcast(&sample_event(i));
    });

    println!();
}
