//! Concurrency stress tests for pylon runtime.
//!
//! These tests verify that all Mutex-based code is safe under contention and
//! that concurrent operations do not lose data, deadlock, or panic.

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::jobs::JobQueue;
use pylon_runtime::rate_limit::RateLimiter;
use pylon_runtime::rooms::RoomManager;
use pylon_runtime::workflows::{WorkflowDef, WorkflowEngine, WorkflowStatus};
use pylon_runtime::Runtime;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn counter_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "concurrency-test".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Counter".into(),
            fields: vec![ManifestField {
                name: "value".into(),
                field_type: "int".into(),
                optional: false,
                unique: false,
            }],
            indexes: vec![],
            relations: vec![],
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
    }
}

// ---------------------------------------------------------------------------
// Runtime CRUD under contention
// ---------------------------------------------------------------------------

/// 10 threads each inserting 100 rows -- all 1000 should survive.
///
/// SQLite serializes writes through the Mutex, so this tests that the lock
/// acquisition is fair and that no inserts are silently dropped.
#[test]
fn concurrent_inserts_dont_lose_data() {
    let rt = Arc::new(Runtime::in_memory(counter_manifest()).unwrap());
    let threads: Vec<_> = (0..10)
        .map(|i| {
            let rt = Arc::clone(&rt);
            thread::spawn(move || {
                for j in 0..100 {
                    rt.insert("Counter", &serde_json::json!({"value": i * 100 + j}))
                        .unwrap();
                }
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    let rows = rt.list("Counter").unwrap();
    assert_eq!(rows.len(), 1000, "all 1000 inserts should succeed");
}

/// Concurrent readers and writers operating on the same row.
///
/// Writers continuously update a row while readers continuously read it.
/// The invariant is that readers never panic and always see a valid snapshot.
#[test]
fn concurrent_reads_and_writes() {
    let rt = Arc::new(Runtime::in_memory(counter_manifest()).unwrap());
    let id = rt
        .insert("Counter", &serde_json::json!({"value": 0}))
        .unwrap();

    let writers: Vec<_> = (0..5)
        .map(|_| {
            let rt = Arc::clone(&rt);
            let id = id.clone();
            thread::spawn(move || {
                for v in 1..=100 {
                    let _ = rt.update("Counter", &id, &serde_json::json!({"value": v}));
                }
            })
        })
        .collect();

    let readers: Vec<_> = (0..5)
        .map(|_| {
            let rt = Arc::clone(&rt);
            let id = id.clone();
            thread::spawn(move || {
                let mut reads = 0u32;
                for _ in 0..100 {
                    if rt.get_by_id("Counter", &id).unwrap().is_some() {
                        reads += 1;
                    }
                }
                reads
            })
        })
        .collect();

    for t in writers {
        t.join().unwrap();
    }
    let total_reads: u32 = readers.into_iter().map(|t| t.join().unwrap()).sum();

    // Every read should find the row (it exists throughout the test).
    assert_eq!(total_reads, 500, "readers should never see a missing row");
}

/// Concurrent filtered queries should not interfere with writers.
#[test]
fn concurrent_filtered_queries_and_inserts() {
    let rt = Arc::new(Runtime::in_memory(counter_manifest()).unwrap());

    let writers: Vec<_> = (0..5)
        .map(|i| {
            let rt = Arc::clone(&rt);
            thread::spawn(move || {
                for j in 0..50 {
                    rt.insert("Counter", &serde_json::json!({"value": i * 50 + j}))
                        .unwrap();
                }
            })
        })
        .collect();

    let readers: Vec<_> = (0..5)
        .map(|_| {
            let rt = Arc::clone(&rt);
            thread::spawn(move || {
                let mut query_count = 0u32;
                for _ in 0..50 {
                    let _ = rt.query_filtered("Counter", &serde_json::json!({"value": {"$gt": 0}}));
                    query_count += 1;
                }
                query_count
            })
        })
        .collect();

    for t in writers {
        t.join().unwrap();
    }
    for t in readers {
        t.join().unwrap();
    }

    let rows = rt.list("Counter").unwrap();
    assert_eq!(rows.len(), 250);
}

// ---------------------------------------------------------------------------
// RoomManager under contention
// ---------------------------------------------------------------------------

/// 20 threads each joining/leaving 10 rooms concurrently.
///
/// After all threads finish (everyone leaves), no rooms should remain.
#[test]
fn concurrent_room_operations() {
    let mgr = Arc::new(RoomManager::new(60));

    let threads: Vec<_> = (0..20)
        .map(|i| {
            let mgr = Arc::clone(&mgr);
            thread::spawn(move || {
                let user = format!("user_{i}");
                for r in 0..10 {
                    let room = format!("room_{r}");
                    let _ = mgr.join(&room, &user, None);
                    mgr.set_presence(&room, &user, serde_json::json!({"active": true}));
                    let _ = mgr.members(&room);
                }
                // Leave all rooms.
                for r in 0..10 {
                    mgr.leave(&format!("room_{r}"), &user);
                }
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    // All rooms should be empty. Some may linger if a leave() raced with
    // another thread's join() on the same room, but every user explicitly
    // left their own membership, so no user should remain.
    for r in 0..10 {
        let room = format!("room_{r}");
        let members = mgr.members(&room);
        assert!(
            members.is_empty(),
            "room {} should be empty but has {} members",
            room,
            members.len()
        );
    }
}

/// Concurrent disconnect() calls should not panic even when multiple
/// threads try to disconnect the same user simultaneously.
#[test]
fn concurrent_disconnect_same_user() {
    let mgr = Arc::new(RoomManager::new(60));

    // Join many rooms first.
    for r in 0..50 {
        mgr.join(&format!("room_{r}"), "alice", None).unwrap();
    }

    let threads: Vec<_> = (0..10)
        .map(|_| {
            let mgr = Arc::clone(&mgr);
            thread::spawn(move || {
                mgr.disconnect("alice");
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    assert!(mgr.user_rooms("alice").is_empty());
}

// ---------------------------------------------------------------------------
// JobQueue under contention
// ---------------------------------------------------------------------------

/// 5 producers each enqueue 100 jobs. A consumer tries to dequeue them all.
/// The total (dequeued + remaining) should equal 500.
#[test]
fn concurrent_job_enqueue_dequeue() {
    let queue = Arc::new(JobQueue::new(1000));

    // Producers
    let producers: Vec<_> = (0..5)
        .map(|i| {
            let q = Arc::clone(&queue);
            thread::spawn(move || {
                for j in 0..100 {
                    q.enqueue(&format!("job_{}_{}", i, j), serde_json::json!({"n": j}));
                }
            })
        })
        .collect();

    // Consumer -- runs concurrently with producers.
    let q = Arc::clone(&queue);
    let consumer = thread::spawn(move || {
        let mut dequeued = 0u32;
        // Try more iterations than produced to handle timing gaps.
        for _ in 0..600 {
            if q.dequeue(Duration::from_millis(5)).is_some() {
                dequeued += 1;
            }
        }
        dequeued
    });

    for t in producers {
        t.join().unwrap();
    }
    let dequeued = consumer.join().unwrap();

    let remaining = queue.pending_count();
    assert_eq!(
        dequeued as usize + remaining,
        500,
        "dequeued ({dequeued}) + remaining ({remaining}) should equal 500"
    );
}

/// Multiple workers processing the same queue should not double-process jobs.
#[test]
fn concurrent_workers_no_double_processing() {
    let queue = Arc::new(JobQueue::new(1000));
    let processed = Arc::new(std::sync::atomic::AtomicU32::new(0));

    // Enqueue 200 jobs.
    for i in 0..200 {
        queue.enqueue(&format!("work_{i}"), serde_json::json!({}));
    }

    // Spin up 8 workers that each try to dequeue.
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let q = Arc::clone(&queue);
            let count = Arc::clone(&processed);
            thread::spawn(move || loop {
                match q.dequeue(Duration::from_millis(10)) {
                    Some(job) => {
                        count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        q.complete(&job.id);
                    }
                    None => break,
                }
            })
        })
        .collect();

    for w in workers {
        w.join().unwrap();
    }

    let total = processed.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(
        total, 200,
        "exactly 200 jobs should be processed, got {total}"
    );
}

// ---------------------------------------------------------------------------
// RateLimiter under contention
// ---------------------------------------------------------------------------

/// Each of 10 threads makes 200 requests from a distinct IP.
/// With a limit of 100 per window, each thread should get exactly 100
/// allowed and 100 denied.
#[test]
fn concurrent_rate_limiter() {
    let limiter = Arc::new(RateLimiter::new(100, 60));

    let threads: Vec<_> = (0..10)
        .map(|i| {
            let limiter = Arc::clone(&limiter);
            thread::spawn(move || {
                let ip = format!("10.0.0.{i}");
                let mut allowed = 0u32;
                let mut denied = 0u32;
                for _ in 0..200 {
                    match limiter.check(&ip) {
                        Ok(()) => allowed += 1,
                        Err(_) => denied += 1,
                    }
                }
                (allowed, denied)
            })
        })
        .collect();

    for t in threads {
        let (allowed, denied) = t.join().unwrap();
        assert_eq!(allowed, 100, "expected 100 allowed, got {allowed}");
        assert_eq!(denied, 100, "expected 100 denied, got {denied}");
    }
}

/// Multiple threads hitting the same IP simultaneously. The total allowed
/// across all threads should still respect the limit.
#[test]
fn concurrent_rate_limiter_same_ip() {
    let limiter = Arc::new(RateLimiter::new(50, 60));

    let threads: Vec<_> = (0..10)
        .map(|_| {
            let limiter = Arc::clone(&limiter);
            thread::spawn(move || {
                let mut allowed = 0u32;
                for _ in 0..100 {
                    if limiter.check("shared-ip").is_ok() {
                        allowed += 1;
                    }
                }
                allowed
            })
        })
        .collect();

    let total_allowed: u32 = threads.into_iter().map(|t| t.join().unwrap()).sum();
    assert_eq!(
        total_allowed, 50,
        "total allowed across all threads should be 50, got {total_allowed}"
    );
}

// ---------------------------------------------------------------------------
// WorkflowEngine under contention
// ---------------------------------------------------------------------------

/// Multiple threads starting and advancing workflows concurrently.
#[test]
fn concurrent_workflow_start_and_advance() {
    let engine = Arc::new(WorkflowEngine::new("http://localhost:19999", 100));
    engine.register(WorkflowDef {
        name: "concurrent_test".into(),
        description: "test".into(),
        file: "test.ts".into(),
        max_retries: 2,
        step_timeout_secs: 30,
    });

    // Start 50 workflows concurrently.
    let starters: Vec<_> = (0..50)
        .map(|i| {
            let e = Arc::clone(&engine);
            thread::spawn(move || {
                e.start("concurrent_test", serde_json::json!({"i": i}))
                    .unwrap()
            })
        })
        .collect();

    let ids: Vec<String> = starters.into_iter().map(|t| t.join().unwrap()).collect();

    // Advance all concurrently with a step_complete response.
    let advancers: Vec<_> = ids
        .iter()
        .map(|id| {
            let e = Arc::clone(&engine);
            let id = id.clone();
            thread::spawn(move || {
                e.advance_with_response(
                    &id,
                    serde_json::json!({
                        "action": "step_complete",
                        "step_name": "init",
                        "output": null
                    }),
                )
                .unwrap()
            })
        })
        .collect();

    for t in advancers {
        let status = t.join().unwrap();
        assert_eq!(status, WorkflowStatus::Running);
    }

    // All workflows should have exactly 1 completed step.
    for id in &ids {
        let wf = engine.get(id).unwrap();
        assert_eq!(wf.steps.len(), 1);
        assert_eq!(wf.current_step, 1);
    }
}
