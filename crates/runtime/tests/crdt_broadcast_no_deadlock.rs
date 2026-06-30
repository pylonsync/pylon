//! Regression: the CRDT-broadcast write path must never hold `write_conn`
//! across `crdt_snapshot`.
//!
//! THE BUG (every chat mutation hung): the SQLite TS-mutation path in
//! `FnOpsImpl::call` held the `write_conn` lock for its BEGIN/COMMIT scope
//! and, after COMMIT, called `broadcast_change_with_crdt` *while still
//! holding the guard*. For a CRDT entity that broadcast calls
//! `Runtime::crdt_snapshot`, which re-acquires `lock_conn_pub()`.
//! `std::sync::Mutex` is NOT reentrant, so the second acquisition on the
//! same thread deadlocked forever. Every `sendMessage` / `createChannel`
//! (Message.body and Channel.topic are `crdt("text")`) hung at the insert.
//!
//! This is the exact shape of the v0.3.219 seq-persistence regression and
//! the May-31 change_log.append reentrancy bug (69c186bb). That fix moved
//! `append` off the held lock but missed this CRDT-broadcast sibling, and
//! shipped without a real-SQLite reentrancy test ("queued behind the
//! broader audit work") — which is precisely why this variant slipped
//! through. This test closes that gap: it pins the invariant the fix
//! relies on directly against a real SQLite `Runtime`.
//!
//! The fix drains the buffered events under the lock, drops `conn_guard`,
//! and only THEN appends + broadcasts + flushes schedules.

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use pylon_http::DataStore;
use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_runtime::Runtime;

/// A single CRDT entity — `crdt: true` is what makes `crdt_snapshot`
/// return `Some(..)` (and what made the broadcast path re-lock).
fn crdt_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "crdt-deadlock".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Todo".into(),
            fields: vec![ManifestField {
                name: "title".into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
            }],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
            sync: true,
        }],
        ..Default::default()
    }
}

#[test]
fn crdt_snapshot_must_not_be_called_while_holding_write_conn() {
    let rt = Arc::new(Runtime::in_memory(crdt_manifest()).expect("runtime"));
    let id = <Runtime as DataStore>::insert(&rt, "Todo", &serde_json::json!({"title": "t"}))
        .expect("insert");

    // FIXED ordering (write_conn free): the broadcast path's snapshot read
    // returns promptly. This is the post-fix mutation path — `conn_guard`
    // is dropped before `broadcast_change_with_crdt` runs.
    let snap =
        <Runtime as DataStore>::crdt_snapshot(&rt, "Todo", &id).expect("snapshot must succeed");
    assert!(
        snap.is_some(),
        "crdt_snapshot returns a Loro frame for a crdt:true entity when the write lock is free",
    );

    // BUGGY ordering (write_conn HELD): reproduce exactly what the old code
    // did — hold the guard, then take the CRDT snapshot. `crdt_snapshot`
    // re-acquires `lock_conn_pub()` on the same thread, so it can never
    // complete. A watchdog proves the hang; the thread is intentionally
    // leaked (it stays parked on the non-reentrant mutex).
    let rt2 = Arc::clone(&rt);
    let id2 = id.clone();
    let (tx, rx) = mpsc::channel::<()>();
    std::thread::spawn(move || {
        let _guard = rt2.lock_conn_pub().expect("hold write_conn");
        // Re-entrant acquisition on the same thread → deadlock. The send
        // below is unreachable for as long as the hazard exists.
        let _ = <Runtime as DataStore>::crdt_snapshot(&rt2, "Todo", &id2);
        let _ = tx.send(());
    });

    assert!(
        rx.recv_timeout(Duration::from_secs(3)).is_err(),
        "crdt_snapshot under a held write_conn MUST deadlock. If it returned, \
         lock_conn_pub() became reentrant and the mutation-path fix (drop \
         conn_guard before broadcasting) is no longer load-bearing — update \
         this test deliberately, don't just delete it.",
    );
}
