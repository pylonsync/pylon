use std::sync::Arc;
use std::time::Instant;

use statecraft_sync::{ChangeEvent, ChangeKind, ChangeLog, SyncCursor};
use statecraft_runtime::ws::WsHub;
use statecraft_runtime::sse::SseHub;

fn bench(name: &str, iterations: u32, f: impl Fn()) {
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
        "  {:<40} {:>8} ops  {:>10.2?} total  {:>8.2?}/op  {:>8} ops/sec",
        name, iterations, elapsed, per_op, ops_sec
    );
}

fn main() {
    println!("\nstatecraft sync benchmarks\n");

    // -- ChangeLog throughput --
    let log = ChangeLog::new();
    bench("changelog append", 100_000, || {
        log.append("User", "u1", ChangeKind::Insert, Some(serde_json::json!({"name":"Alice"})));
    });

    // -- Pull performance with large log --
    let log = ChangeLog::new();
    for i in 0..10_000 {
        log.append("User", &format!("u{i}"), ChangeKind::Insert, Some(serde_json::json!({"name":"User"})));
    }

    bench("pull 100 from 10k log", 10_000, || {
        let _ = log.pull(&SyncCursor { last_seq: 9900 }, 100);
    });

    bench("pull 1000 from 10k log", 1_000, || {
        let _ = log.pull(&SyncCursor { last_seq: 9000 }, 1000);
    });

    bench("pull all 10k", 100, || {
        let _ = log.pull(&SyncCursor::beginning(), 10_000);
    });

    // -- WsHub broadcast (no clients) --
    let hub = WsHub::new();
    let event = ChangeEvent {
        seq: 1,
        entity: "User".into(),
        row_id: "u1".into(),
        kind: ChangeKind::Insert,
        data: Some(serde_json::json!({"name":"Alice","email":"alice@test.com"})),
        timestamp: "2024-01-01T00:00:00Z".into(),
    };

    bench("ws broadcast (0 clients)", 100_000, || {
        hub.broadcast(&event);
    });

    // -- SseHub broadcast (no clients) --
    let sse = SseHub::new();
    bench("sse broadcast (0 clients)", 100_000, || {
        sse.broadcast(&event);
    });

    // -- Simulated 10k connected clients scenario --
    // We can't create real WebSocket connections in a bench,
    // but we can measure the serialization + lock overhead.
    println!();
    println!("  Simulated 10k client broadcast:");

    let event_json = serde_json::to_string(&event).unwrap();
    let event_bytes = event_json.len();
    println!("    Event size: {} bytes", event_bytes);

    // Measure serialization time for 10k broadcasts.
    let start = Instant::now();
    for _ in 0..10_000 {
        let _json = serde_json::to_string(&event).unwrap();
    }
    let ser_time = start.elapsed();
    println!("    Serialize 10k events: {:?} ({:.2?}/event)", ser_time, ser_time / 10_000);

    // Measure lock contention under simulated load.
    let hub = WsHub::new();
    let hub = Arc::new(hub);
    let start = Instant::now();
    let mut handles = vec![];
    for _ in 0..10 {
        let h = Arc::clone(&hub);
        let e = event.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..1_000 {
                h.broadcast(&e);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let concurrent_time = start.elapsed();
    println!("    10 threads × 1000 broadcasts: {:?}", concurrent_time);

    // Estimate: at 10k clients, each broadcast needs to write to 10k sockets.
    // With ~100 bytes per event and TCP buffering:
    // - Serialization: ~500ns/event (amortized, serialize once)
    // - Socket write: ~1-10µs per client (kernel buffered)
    // - 10k clients: 10-100ms per broadcast
    // - At 100 mutations/sec: need to broadcast 100 events/sec to 10k clients
    // - That's 1M socket writes/sec — feasible with batching
    println!();
    println!("  Estimated 10k client capacity:");
    println!("    Single broadcast to 10k: ~10-100ms (TCP buffered writes)");
    println!("    Max sustained mutation rate: ~10-100/sec at 10k clients");
    println!("    Recommendation: batch broadcasts at >1k clients");

    println!();
}
