use std::time::Instant;

use pylon_kernel::*;
use pylon_runtime::Runtime;

fn test_manifest() -> AppManifest {
    serde_json::from_str(include_str!("../../../examples/todo-app/pylon.manifest.json")).unwrap()
}

fn bench(name: &str, iterations: u32, f: impl Fn()) {
    // Warmup.
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
        "  {:<30} {:>8} ops  {:>10.2?} total  {:>8.2?}/op  {:>8} ops/sec",
        name, iterations, elapsed, per_op, ops_sec
    );
}

fn main() {
    println!("\npylon runtime benchmarks\n");

    let rt = Runtime::in_memory(test_manifest()).unwrap();

    // -- Insert --
    bench("insert (User)", 10_000, || {
        let _ = rt.insert(
            "User",
            &serde_json::json!({
                "email": format!("user{}@test.com", rand()),
                "displayName": "Test User",
                "createdAt": "2024-01-01T00:00:00Z"
            }),
        );
    });

    // Seed some data for reads.
    let rt = Runtime::in_memory(test_manifest()).unwrap();
    let mut ids = Vec::new();
    for i in 0..1000 {
        let id = rt
            .insert(
                "User",
                &serde_json::json!({
                    "email": format!("user{i}@test.com"),
                    "displayName": format!("User {i}"),
                    "createdAt": "2024-01-01T00:00:00Z"
                }),
            )
            .unwrap();
        ids.push(id);
    }

    // -- Get by ID --
    let id = ids[500].clone();
    bench("get_by_id (User)", 10_000, || {
        let _ = rt.get_by_id("User", &id);
    });

    // -- List --
    bench("list (1000 Users)", 1_000, || {
        let _ = rt.list("User");
    });

    // -- Lookup by field --
    bench("lookup (User by email)", 10_000, || {
        let _ = rt.lookup("User", "email", "user500@test.com");
    });

    // -- Filtered query --
    bench("query_filtered (equality)", 1_000, || {
        let _ = rt.query_filtered("User", &serde_json::json!({"displayName": "User 500"}));
    });

    bench("query_filtered ($like)", 1_000, || {
        let _ = rt.query_filtered("User", &serde_json::json!({"email": {"$like": "user5"}}));
    });

    bench("query_filtered ($order + $limit)", 1_000, || {
        let _ = rt.query_filtered(
            "User",
            &serde_json::json!({"$order": {"displayName": "asc"}, "$limit": 10}),
        );
    });

    // -- Update --
    let id = ids[0].clone();
    bench("update (User)", 10_000, || {
        let _ = rt.update("User", &id, &serde_json::json!({"displayName": "Updated"}));
    });

    // -- Delete + reinsert --
    bench("delete (User)", 1_000, || {
        let id = rt
            .insert(
                "User",
                &serde_json::json!({
                    "email": format!("del{}@test.com", rand()),
                    "displayName": "Delete Me",
                    "createdAt": "2024-01-01T00:00:00Z"
                }),
            )
            .unwrap();
        let _ = rt.delete("User", &id);
    });

    // -- Graph query --
    bench("query_graph (User)", 1_000, || {
        let _ = rt.query_graph(&serde_json::json!({"User": {}}));
    });

    bench("query_graph (User with where)", 1_000, || {
        let _ = rt.query_graph(&serde_json::json!({"User": {"where": {"displayName": "User 500"}}}));
    });

    // -- Insert Todo with all fields --
    let rt2 = Runtime::in_memory(test_manifest()).unwrap();
    bench("insert (Todo, all fields)", 10_000, || {
        let _ = rt2.insert(
            "Todo",
            &serde_json::json!({
                "title": "Buy milk",
                "done": false,
                "authorId": "user-1",
                "createdAt": "2024-01-01T00:00:00Z"
            }),
        );
    });

    println!();
}

fn rand() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    t.as_nanos() as u64
}
