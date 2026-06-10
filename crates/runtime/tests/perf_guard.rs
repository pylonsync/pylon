//! Catastrophic-performance-regression guard rails.
//!
//! These are NOT precise microbenchmarks — those live in `benches/` +
//! `docs/ops/SIZING.md` and are run on demand. These run in the normal
//! `cargo test` (and a dedicated CI `perf-guard` job), so every PR gates on
//! them. Each asserts a core data-plane op stays within a GENEROUS wall-clock
//! bound — roughly 20–100× the reference-laptop numbers in SIZING.md — so a
//! slow, shared CI runner won't flake, but an ALGORITHMIC blowup (a dropped
//! index turning a lookup into a full scan, an O(n)→O(n²) query, a lock
//! storm, an accidental clone-per-row) trips the alarm loudly with a message
//! pointing at the regressed operation.
//!
//! If one fails: it is almost never "the machine was slow" (the bounds have
//! 20×+ headroom) — it's a real complexity regression. Profile the named op.
//!
//! RELEASE ONLY: the bounds are calibrated against the release-build SIZING.md
//! numbers, and debug Rust is ~10–30× slower (which would false-positive). So
//! the whole file compiles out of debug `cargo test`; the dedicated CI
//! `perf-guard` job runs `cargo test --release --test perf_guard`.
#![cfg(not(debug_assertions))]

use std::time::{Duration, Instant};

use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;

fn manifest() -> AppManifest {
    serde_json::from_str(include_str!(
        "../../../examples/todo-app/pylon.manifest.json"
    ))
    .unwrap()
}

/// Fresh in-memory runtime seeded with `n` User rows; returns the row ids.
fn seeded(n: usize) -> (Runtime, Vec<String>) {
    let rt = Runtime::in_memory(manifest()).unwrap();
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let id = rt
            .insert(
                "User",
                &serde_json::json!({
                    "email": format!("user{i}@test.com"),
                    "displayName": format!("User {i}"),
                    "createdAt": "2024-01-01T00:00:00Z",
                }),
            )
            .unwrap();
        ids.push(id);
    }
    (rt, ids)
}

/// Run `f` `iters` times and assert it finished under `budget`.
fn within(name: &str, reference: &str, iters: u32, budget: Duration, f: impl Fn()) {
    // A couple of warmups so first-call lazy init (prepared-stmt cache, etc.)
    // doesn't dominate a low-iteration guard.
    f();
    f();
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    let e = t.elapsed();
    assert!(
        e < budget,
        "PERF REGRESSION: {name} ({iters} ops) took {e:?}, budget {budget:?} \
         (reference ≈ {reference}). The bound has ~20×+ headroom for slow CI, \
         so this is almost certainly an algorithmic regression — profile {name}.",
    );
}

#[test]
fn perf_guard_insert_5k() {
    let rt = Runtime::in_memory(manifest()).unwrap();
    let t = Instant::now();
    for i in 0..5_000 {
        rt.insert(
            "User",
            &serde_json::json!({
                "email": format!("ins{i}@t.com"),
                "displayName": "x",
                "createdAt": "2024-01-01T00:00:00Z",
            }),
        )
        .unwrap();
    }
    let e = t.elapsed();
    // SIZING: ~68k inserts/sec → 5k ≈ 75ms. Budget 5s (~66×).
    assert!(
        e < Duration::from_secs(5),
        "PERF REGRESSION: 5k inserts took {e:?} (reference ≈ 75ms); profile insert.",
    );
}

#[test]
fn perf_guard_get_by_id() {
    let (rt, ids) = seeded(1000);
    let id = ids[500].clone();
    // SIZING: ~519k get_by_id/sec → 10k ≈ 19ms. Budget 2s (~100×).
    within("get_by_id", "19ms", 10_000, Duration::from_secs(2), || {
        let _ = rt.get_by_id("User", &id);
    });
}

#[test]
fn perf_guard_lookup_by_unique_field() {
    // A unique field is indexed; a regressed index → O(n) scan would blow this
    // up (10k scans × 1k rows). SIZING: ~484k/sec → 10k ≈ 21ms. Budget 3s.
    let (rt, _) = seeded(1000);
    within("lookup", "21ms", 10_000, Duration::from_secs(3), || {
        let _ = rt.lookup("User", "email", "user500@test.com");
    });
}

#[test]
fn perf_guard_filtered_query_equality() {
    // SIZING: ~24k/sec → 1k ≈ 41ms. Budget 5s (~120×).
    let (rt, _) = seeded(1000);
    within(
        "query_filtered(eq)",
        "41ms",
        1_000,
        Duration::from_secs(5),
        || {
            let _ = rt.query_filtered("User", &serde_json::json!({"displayName": "User 500"}));
        },
    );
}

#[test]
fn perf_guard_list_1k() {
    // SIZING: list(1000) ≈ 363µs → 500× ≈ 180ms. Budget 5s (~27×).
    let (rt, _) = seeded(1000);
    within("list(1000)", "180ms", 500, Duration::from_secs(5), || {
        let _ = rt.list("User");
    });
}

#[test]
fn perf_guard_change_log_append_100k() {
    use pylon_sync::{ChangeKind, ChangeLog};
    let log = ChangeLog::new();
    let t = Instant::now();
    for i in 0..100_000u64 {
        log.append("Note", &format!("n{i}"), ChangeKind::Insert, None);
    }
    let e = t.elapsed();
    // SIZING: ~5M append/sec → 100k ≈ 20ms. Budget 3s (~150×). Catches a
    // regression that made the in-memory ring/seq path super-linear.
    assert!(
        e < Duration::from_secs(3),
        "PERF REGRESSION: 100k change_log.append took {e:?} (reference ≈ 20ms); profile append.",
    );
}
