use pylon_kernel::AppManifest;
use pylon_runtime::Runtime;
use pylon_sync::{ChangeEvent, ChangeKind};
use serde_json::json;
use std::sync::{mpsc, Arc};
use std::time::Duration;

fn seed(runtime: &Runtime) {
    runtime.bootstrap_sqlite_change_log().unwrap();
    let events: Vec<_> = (1..=4)
        .map(|seq| ChangeEvent {
            seq,
            entity: "Item".into(),
            row_id: format!("row-{seq}"),
            kind: ChangeKind::Insert,
            data: Some(json!({"id": format!("row-{seq}")})),
            prev_data: None,
            timestamp: "2026-10-08T00:00:00Z".into(),
        })
        .collect();
    runtime.sqlite_change_log_persist_batch(&events).unwrap();
}

fn check_reads(runtime: &Runtime) {
    assert!(runtime.sqlite_change_log_has_entity("Item"));
    assert!(!runtime.sqlite_change_log_has_entity("Missing"));
    let recent = runtime.sqlite_change_log_load_recent(2);
    assert_eq!(recent.iter().map(|e| e.seq).collect::<Vec<_>>(), [3, 4]);
    let replay = runtime.sqlite_change_log_pull_range(1, 2).unwrap();
    assert_eq!(replay.iter().map(|e| e.seq).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(replay[0].data, Some(json!({"id": "row-2"})));
    assert!(runtime
        .sqlite_change_log_pull_range(4, 2)
        .unwrap()
        .is_empty());
}

#[test]
fn replay_reads_do_not_wait_for_the_writer() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Arc::new(
        Runtime::open(
            dir.path().join("replay.sqlite").to_str().unwrap(),
            AppManifest::default(),
        )
        .unwrap(),
    );
    seed(&runtime);
    assert!(runtime.read_pool_size() > 0);
    let writer = runtime.lock_conn_pub().unwrap();
    writer
        .execute_batch("BEGIN; UPDATE _pylon_change_log SET data = '{}' WHERE seq = 2;")
        .unwrap();
    let reader = Arc::clone(&runtime);
    let (send, receive) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        check_reads(&reader);
        send.send(()).unwrap();
    });
    let result = receive.recv_timeout(Duration::from_secs(5));
    // Release before asserting so a regression cannot leave a blocked worker.
    writer.execute_batch("ROLLBACK").unwrap();
    drop(writer);
    worker.join().unwrap();
    result.expect("replay reads must use the reader pool");
}

#[test]
fn replay_reads_keep_the_in_memory_fallback_and_missing_table_errors() {
    let runtime = Runtime::in_memory(AppManifest::default()).unwrap();
    assert!(runtime.sqlite_change_log_pull_range(0, 2).is_none());
    seed(&runtime);
    check_reads(&runtime);
}
