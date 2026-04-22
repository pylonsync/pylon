//! Fuzz and property-based tests for statecraft parsers and data structures.
//!
//! These tests feed arbitrary, malformed, and edge-case inputs to parsers
//! and data structure APIs to verify they never panic -- they should return
//! `Ok` or `Err`, but never unwind.

use std::io::{BufReader, Cursor};

use statecraft_plugin::builtin::cache::CachePlugin;
use statecraft_plugin::builtin::file_storage::FileStoragePlugin;
use statecraft_runtime::cron::CronExpr;
use statecraft_runtime::resp::parse_resp;
use statecraft_runtime::workflows::{WorkflowDef, WorkflowEngine, WorkflowStatus};

// ---------------------------------------------------------------------------
// RESP parser -- must never panic on arbitrary byte sequences
// ---------------------------------------------------------------------------

#[test]
fn resp_parser_doesnt_panic_on_garbage() {
    let inputs: Vec<&[u8]> = vec![
        b"",
        b"\r\n",
        b"garbage",
        b"+\r\n",
        b"$-1\r\n",
        b"*-1\r\n",
        b"$999999999\r\n",
        b"*0\r\n",
        b":\r\n",
        b":abc\r\n",
        b"+OK",           // missing \r\n
        b"-ERR",          // missing \r\n
        b"$0\r\n\r\n",
        b"\x00\x01\x02\x03",
        b"+++\r\n",
        b"---\r\n",
        b"$2\r\nab\r\n",
        b"$2\r\na",       // truncated bulk string
        b"*2\r\n+OK\r\n", // incomplete array (only 1 of 2 elements)
        b"*1\r\n*1\r\n*1\r\n+deep\r\n", // nested arrays
    ];

    // Large inputs that might cause allocation issues.
    let star_repeat = "*".repeat(1000);
    let plus_repeat = "+".repeat(10000);

    for input in inputs {
        let mut reader = BufReader::new(Cursor::new(input));
        let _ = parse_resp(&mut reader); // must not panic
    }

    for large in [star_repeat.as_bytes(), plus_repeat.as_bytes()] {
        let mut reader = BufReader::new(Cursor::new(large));
        let _ = parse_resp(&mut reader);
    }
}

/// Roundtrip property: any successfully parsed RESP value should serialize
/// back to bytes that parse to the same value.
#[test]
fn resp_roundtrip_property() {
    use statecraft_runtime::resp::RespValue;

    let values = vec![
        RespValue::SimpleString(String::new()),
        RespValue::SimpleString("hello world".into()),
        RespValue::Error("ERR bad".into()),
        RespValue::Integer(0),
        RespValue::Integer(-1),
        RespValue::Integer(i64::MAX),
        RespValue::Integer(i64::MIN),
        RespValue::BulkString(None),
        RespValue::BulkString(Some(String::new())),
        RespValue::BulkString(Some("x".repeat(10_000))),
        RespValue::Array(None),
        RespValue::Array(Some(vec![])),
        RespValue::Array(Some(vec![
            RespValue::Integer(1),
            RespValue::BulkString(Some("two".into())),
            RespValue::BulkString(None),
        ])),
    ];

    for val in &values {
        let bytes = val.serialize();
        let mut reader = BufReader::new(Cursor::new(&bytes));
        let parsed = parse_resp(&mut reader).expect("roundtrip parse should succeed");
        assert_eq!(&parsed, val, "roundtrip mismatch for {val:?}");
    }
}

// ---------------------------------------------------------------------------
// Cron parser -- must never panic on arbitrary expressions
// ---------------------------------------------------------------------------

#[test]
fn cron_parser_doesnt_panic_on_garbage() {
    let inputs = vec![
        "",
        "* * * * *",
        "*/0 * * * *",
        "99 99 99 99 99",
        "-1 * * * *",
        "a b c d e",
        "* * * *",       // too few fields
        "* * * * * *",   // too many fields
        "0-60 * * * *",  // range exceeds max
        "*/abc * * * *",
        ",,,, * * * *",
        "1-2-3 * * * *",
        "0, 5, 10 * * * *", // spaces after commas
        "   ",
        "\n\t",
        "0 0 31 2 *", // Feb 31 -- valid cron, just never fires
    ];

    let star_repeat = "*".repeat(100);

    for input in inputs {
        let _ = CronExpr::parse(input); // Ok or Err, never panic
    }
    let _ = CronExpr::parse(&star_repeat);
}

/// Edge-case timestamps fed to a valid cron expression should not panic.
#[test]
fn cron_matches_edge_timestamps() {
    let cron = CronExpr::parse("* * * * *").unwrap();

    let timestamps: Vec<u64> = vec![
        0,
        1,
        86400,
        86400 * 365 * 50, // ~50 years
        1_000_000_000,    // ~2001
        2_000_000_000,    // ~2033
        u64::MAX / 2,
        // u64::MAX would overflow in decompose_timestamp but the function
        // should not panic (it casts to i64 which wraps).
    ];

    for ts in timestamps {
        let _ = cron.matches(ts); // must not panic
    }
}

// ---------------------------------------------------------------------------
// Cache plugin -- must handle weird keys and concurrent INCR
// ---------------------------------------------------------------------------

#[test]
fn cache_doesnt_panic_on_weird_keys() {
    let cache = CachePlugin::new(1000);

    let weird_keys: Vec<String> = vec![
        String::new(),
        " ".into(),
        "\0".into(),
        "\n\r\t".into(),
        "a".repeat(10_000),
        "key with spaces".into(),
        "key\0null".into(),
        // Unicode
        "\u{65e5}\u{672c}\u{8a9e}\u{30ad}\u{30fc}".into(),
        // Injection attempts
        "../../../etc/passwd".into(),
        "key;DROP TABLE".into(),
    ];

    for key in &weird_keys {
        cache.set(key, "value", None);
        let _ = cache.get(key);
        let _ = cache.incr(key); // may Err if value is not numeric, that's fine
        cache.del(key);
        let _ = cache.exists(key);
        let _ = cache.ttl(key);
        let _ = cache.key_type(key);
    }
    // Reaching here without a panic is the test.
}

/// Concurrent INCR on a single key must produce an exact total.
///
/// 10 threads x 1000 increments = 10000.
#[test]
fn concurrent_incr_atomicity() {
    use std::sync::Arc;
    use std::thread;

    let cache = Arc::new(CachePlugin::new(100_000));

    let threads: Vec<_> = (0..10)
        .map(|_| {
            let cache = Arc::clone(&cache);
            thread::spawn(move || {
                for _ in 0..1000 {
                    cache.incr("atomic_counter").unwrap();
                }
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    let val = cache.get("atomic_counter").unwrap();
    assert_eq!(val, "10000", "expected 10000, got {val}");
}

/// Concurrent mixed cache operations (set/get/del) should not deadlock.
#[test]
fn concurrent_cache_mixed_ops() {
    use std::sync::Arc;
    use std::thread;

    let cache = Arc::new(CachePlugin::new(100_000));

    let threads: Vec<_> = (0..10)
        .map(|i| {
            let cache = Arc::clone(&cache);
            thread::spawn(move || {
                for j in 0..1000 {
                    let key = format!("key_{}_{}", i, j);
                    cache.set(&key, "value", None);
                    let _ = cache.get(&key);
                    if j % 3 == 0 {
                        cache.del(&key);
                    }
                }
            })
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    // Should not have panicked or deadlocked.
    assert!(cache.dbsize() > 0);
}

// ---------------------------------------------------------------------------
// File storage -- path traversal variants must not access the filesystem
// ---------------------------------------------------------------------------

#[test]
fn file_storage_rejects_traversal_variants() {
    let dir = std::env::temp_dir().join("statecraft_fuzz_file_storage");
    let storage = FileStoragePlugin::local(&dir).unwrap();

    let bad_ids = vec![
        "../etc/passwd",
        "..\\windows\\system32",
        "foo/../bar",
        "foo/bar",
        "foo\\bar",
        ".hidden",
        "..dotdot",
        "%2e%2e/etc/passwd",
    ];

    for id in bad_ids {
        let result = storage.download(id);
        // Should return Err, never panic, and never actually read from the filesystem
        // outside the storage directory.
        assert!(
            result.is_err(),
            "download({id:?}) should be rejected but returned Ok"
        );
    }

    // A "normal" ID should not panic either (will be not-found since we
    // didn't upload anything).
    let result = storage.download("normal_file");
    assert!(result.is_err()); // not found, but no panic
}

// ---------------------------------------------------------------------------
// Workflow state machine -- transitions must always be valid
// ---------------------------------------------------------------------------

/// Verify that the retry counter correctly transitions a workflow to Failed
/// after exhausting max_retries.
#[test]
fn workflow_state_machine_retry_exhaustion() {
    let engine = WorkflowEngine::new("http://localhost:19999", 100);
    engine.register(WorkflowDef {
        name: "retry_test".into(),
        description: "test".into(),
        file: "test.ts".into(),
        max_retries: 2,
        step_timeout_secs: 30,
    });

    let id = engine
        .start("retry_test", serde_json::json!({}))
        .unwrap();

    // First 2 failures should keep the workflow Running (retrying).
    for i in 0..2 {
        let status = engine
            .advance_with_response(
                &id,
                serde_json::json!({
                    "action": "fail",
                    "step_name": "flaky",
                    "error": format!("attempt {i}")
                }),
            )
            .unwrap();
        assert_eq!(
            status,
            WorkflowStatus::Running,
            "retry {i} should keep running"
        );
    }

    // 3rd failure exhausts retries -- should transition to Failed.
    let status = engine
        .advance_with_response(
            &id,
            serde_json::json!({
                "action": "fail",
                "step_name": "flaky",
                "error": "final"
            }),
        )
        .unwrap();
    assert_eq!(status, WorkflowStatus::Failed);

    let wf = engine.get(&id).unwrap();
    assert_eq!(wf.status, WorkflowStatus::Failed);
    assert_eq!(wf.error, Some("final".into()));
}

/// Once a workflow reaches a terminal state (Completed, Failed, Cancelled),
/// further advance calls should be no-ops returning the terminal status.
#[test]
fn workflow_terminal_states_are_idempotent() {
    let engine = WorkflowEngine::new("http://localhost:19999", 100);
    engine.register(WorkflowDef {
        name: "terminal_test".into(),
        description: "test".into(),
        file: "test.ts".into(),
        max_retries: 0,
        step_timeout_secs: 30,
    });

    // Test Completed.
    let id = engine.start("terminal_test", serde_json::json!({})).unwrap();
    engine
        .advance_with_response(&id, serde_json::json!({"action": "complete", "output": 42}))
        .unwrap();

    let status = engine
        .advance_with_response(
            &id,
            serde_json::json!({"action": "step_complete", "step_name": "ignored"}),
        )
        .unwrap();
    assert_eq!(status, WorkflowStatus::Completed);

    // Test Cancelled.
    let id2 = engine.start("terminal_test", serde_json::json!({})).unwrap();
    engine.cancel(&id2).unwrap();

    let status = engine
        .advance_with_response(
            &id2,
            serde_json::json!({"action": "step_complete", "step_name": "ignored"}),
        )
        .unwrap();
    assert_eq!(status, WorkflowStatus::Cancelled);

    // Test Failed.
    let id3 = engine.start("terminal_test", serde_json::json!({})).unwrap();
    engine
        .advance_with_response(
            &id3,
            serde_json::json!({"action": "fail", "step_name": "s", "error": "boom"}),
        )
        .unwrap();

    let status = engine
        .advance_with_response(
            &id3,
            serde_json::json!({"action": "step_complete", "step_name": "ignored"}),
        )
        .unwrap();
    assert_eq!(status, WorkflowStatus::Failed);
}

/// Feed various action types in sequence to exercise all state transitions.
#[test]
fn workflow_mixed_action_sequence() {
    let engine = WorkflowEngine::new("http://localhost:19999", 100);
    engine.register(WorkflowDef {
        name: "mixed_test".into(),
        description: "test".into(),
        file: "test.ts".into(),
        max_retries: 1,
        step_timeout_secs: 30,
    });

    let id = engine.start("mixed_test", serde_json::json!({})).unwrap();

    let responses = vec![
        serde_json::json!({"action": "step_complete", "step_name": "s1", "output": null}),
        serde_json::json!({"action": "sleep", "duration": "0s"}), // immediate wake
        serde_json::json!({"action": "step_complete", "step_name": "s2", "output": "ok"}),
        serde_json::json!({"action": "fail", "step_name": "s3", "error": "oops"}),
        // retry of s3 succeeds
        serde_json::json!({"action": "step_complete", "step_name": "s3", "output": "recovered"}),
        serde_json::json!({"action": "complete", "output": {"done": true}}),
    ];

    for resp in responses {
        let status = engine.advance_with_response(&id, resp);
        // Should never panic -- Ok or Err.
        assert!(status.is_ok() || status.is_err());
    }

    let wf = engine.get(&id).unwrap();
    assert_eq!(wf.status, WorkflowStatus::Completed);
}

/// Malformed action responses should produce errors, not panics.
#[test]
fn workflow_malformed_responses_dont_panic() {
    let engine = WorkflowEngine::new("http://localhost:19999", 100);
    engine.register(WorkflowDef {
        name: "malformed_test".into(),
        description: "test".into(),
        file: "test.ts".into(),
        max_retries: 0,
        step_timeout_secs: 30,
    });

    let _id = engine
        .start("malformed_test", serde_json::json!({}))
        .unwrap();

    let malformed_responses = vec![
        serde_json::json!({}),                        // no action field
        serde_json::json!({"action": null}),          // null action
        serde_json::json!({"action": 42}),            // non-string action
        serde_json::json!({"action": ""}),            // empty action
        serde_json::json!({"action": "nonexistent"}), // unknown action
        serde_json::json!({"action": "sleep"}),       // sleep without duration
        serde_json::json!({"action": "fail"}),        // fail without error/step_name
        serde_json::json!([1, 2, 3]),                 // array instead of object
        serde_json::json!("just a string"),
        serde_json::json!(42),
    ];

    for resp in malformed_responses {
        // Re-start a fresh workflow for each test since the previous one
        // may have transitioned to a terminal state.
        let test_id = engine
            .start("malformed_test", serde_json::json!({}))
            .unwrap();
        let result = engine.advance_with_response(&test_id, resp);
        // Must not panic. Ok or Err are both acceptable.
        let _ = result;
    }
}
