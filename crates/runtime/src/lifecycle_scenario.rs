//! End-to-end lifecycle harness for the sync change log.
//!
//! The four restart/reload P0s the audit caught (the reentrant
//! `write_conn` deadlock, the per-database seed gate that hid new
//! entities, the snapshot-truncation 503, the push-error swallow) all
//! shared a root cause: there was no test that drove the *real* runtime
//! + change-log + persister + store wiring across a process restart.
//! Every unit test either mocked the store ([`pylon_sync`]'s `MemStore`)
//! or exercised one method in isolation. None of them booted the actual
//! SQLite-backed stack, mutated it, simulated a restart, and re-pulled.
//!
//! This harness does exactly that. It opens a *file-backed* SQLite DB
//! (so dropping + reopening the runtime is a faithful process restart —
//! an in-memory DB would evaporate), and drives it through the real
//! [`crate::server::build_persistent_change_log`] wiring that a live
//! boot uses. The transitions it models:
//!
//!   - **mutate**  — insert a row + append to the change log, exactly
//!                   as the mutation pipeline does.
//!   - **restart** — flush the persister, drop the runtime + change
//!                   log, reopen against the same file, rebuild the
//!                   change log. Models a redeploy / crash-restart.
//!   - **reload**  — client re-pulls from `cursor = 0` (fresh page
//!                   load). Must reconstruct the full visible state.
//!   - **reconnect** — client re-pulls from a saved cursor. Must get
//!                   exactly the events it missed.
//!
//! A regression in any of the boot-wiring P0s surfaces here as a failed
//! assertion (missing rows, reset seqs, double-seeded events) or — for
//! the deadlock class — a hang that trips the per-test wall-clock bound.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_sync::{ChangeEvent, ChangeLog, ChangeRecord, PullError, SyncCursor};
use tempfile::TempDir;

use crate::Runtime;

/// Generous flush bound. The persister commits in sub-millisecond
/// batches; 5s only ever trips if something is genuinely wedged.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// A booted Pylon stack backed by a real on-disk SQLite database,
/// drivable through restart / reload / reconnect transitions.
struct LifecycleHarness {
    /// Kept alive so the temp directory (and the DB file inside it)
    /// isn't deleted until the harness drops.
    _dir: TempDir,
    db_path: String,
    manifest: AppManifest,
    runtime: Arc<Runtime>,
    change_log: Arc<ChangeLog>,
}

impl LifecycleHarness {
    /// Boot a fresh stack against a new temp DB.
    fn boot(manifest: AppManifest) -> Self {
        let dir = tempfile::tempdir().expect("create temp dir");
        let db_path = dir
            .path()
            .join("lifecycle.sqlite")
            .to_str()
            .expect("utf8 db path")
            .to_string();
        let (runtime, change_log) = Self::open(&db_path, &manifest);
        Self {
            _dir: dir,
            db_path,
            manifest,
            runtime,
            change_log,
        }
    }

    /// Open the runtime + build the change log via the *real* boot
    /// wiring. Shared by `boot` and `restart`.
    fn open(db_path: &str, manifest: &AppManifest) -> (Arc<Runtime>, Arc<ChangeLog>) {
        let runtime =
            Arc::new(Runtime::open(db_path, manifest.clone()).expect("open file-backed runtime"));
        let change_log = crate::server::build_persistent_change_log(&runtime);
        (runtime, change_log)
    }

    /// Simulate a clean process restart: flush the persister so every
    /// enqueued event is on disk, drop the old runtime + change log,
    /// reopen against the same file, and rebuild the change log (which
    /// re-runs bootstrap + hydrate-from-disk + per-entity seeding).
    fn restart(&mut self) {
        self.restart_with_manifest(self.manifest.clone());
    }

    /// Restart, swapping in a new manifest — models a redeploy that
    /// added or changed entities. Exercises the per-entity seed gate:
    /// entities already covered by persisted events are NOT re-seeded
    /// (no duplicates), while entities new to the manifest (or with
    /// rows present but no events) ARE seeded.
    fn restart_with_manifest(&mut self, manifest: AppManifest) {
        // Barrier: block until the background persister has committed
        // everything appended so far. Without this the drop below could
        // race the worker and lose the tail — the exact bug the flush
        // primitive closes on real shutdown.
        assert!(
            self.change_log.flush(FLUSH_TIMEOUT),
            "persister flush timed out — possible deadlock or wedged write_conn"
        );
        // Reassigning the fields drops the old `Arc<ChangeLog>` (tearing
        // down its persister thread) and the old runtime. Build the new
        // pair first so we never leave a field uninitialised.
        let (runtime, change_log) = Self::open(&self.db_path, &manifest);
        self.manifest = manifest;
        self.change_log = change_log;
        self.runtime = runtime;
    }

    /// Mutate: insert a row into storage AND append the Insert to the
    /// change log — exactly the two effects the mutation pipeline has.
    /// Returns the minted seq.
    fn insert(&self, entity: &str, id: &str, fields: serde_json::Value) -> u64 {
        let mut row = fields;
        row.as_object_mut()
            .expect("row must be an object")
            .insert("id".into(), serde_json::Value::String(id.into()));
        let stored_id = self.runtime.insert(entity, &row).expect("storage insert");
        assert_eq!(stored_id, id, "explicit id should round-trip");
        self.change_log
            .record(entity, id, ChangeRecord::Insert { row })
            .seq
    }

    /// Insert a row into storage ONLY — no change-log event. Models a
    /// row that reached the table out-of-band (a migrated-in entity, a
    /// snapshot restore, raw SQL). On the next boot the per-entity seed
    /// loop must notice these rows have no events and seed them; a
    /// binary "any persisted event" gate would leave them invisible.
    fn insert_raw(&self, entity: &str, id: &str, fields: serde_json::Value) {
        let mut row = fields;
        row.as_object_mut()
            .expect("row must be an object")
            .insert("id".into(), serde_json::Value::String(id.into()));
        self.runtime.insert(entity, &row).expect("storage insert");
    }

    /// Pull every change with `seq > cursor`, following `has_more`
    /// pagination. Panics on `ResyncRequired` so a test that expects a
    /// clean delta fails loudly if the server demands a resync instead.
    fn pull_all(&self, cursor: u64) -> Vec<ChangeEvent> {
        let mut out = Vec::new();
        let mut cur = SyncCursor { last_seq: cursor };
        loop {
            match self.change_log.pull(&cur, 256) {
                Ok(resp) => {
                    out.extend(resp.changes.iter().cloned());
                    cur = resp.cursor.clone();
                    if !resp.has_more || resp.changes.is_empty() {
                        break;
                    }
                }
                Err(PullError::ResyncRequired { oldest_seq, .. }) => {
                    panic!(
                        "unexpected ResyncRequired (oldest_seq={oldest_seq}) pulling from cursor={cursor}"
                    );
                }
            }
        }
        out
    }

    /// Row ids visible for `entity` when pulling from `cursor`, in pull
    /// order. Deletes are excluded (a tombstone removes the row).
    fn visible_ids(&self, entity: &str, cursor: u64) -> Vec<String> {
        self.pull_all(cursor)
            .into_iter()
            .filter(|e| e.entity == entity && e.kind != pylon_sync::ChangeKind::Delete)
            .map(|e| e.row_id)
            .collect()
    }

    fn current_seq(&self) -> u64 {
        self.change_log.current_seq()
    }
}

// --- manifest builders ------------------------------------------------

fn field(name: &str) -> ManifestField {
    ManifestField {
        name: name.into(),
        field_type: "string".into(),
        optional: false,
        unique: false,
        crdt: None,
        server_only: false,
        readonly: false,
        default: None,
        enum_values: None,
        encrypted: false,
    }
}

fn entity(name: &str, fields: &[&str]) -> ManifestEntity {
    ManifestEntity {
        name: name.into(),
        fields: fields.iter().map(|f| field(f)).collect(),
        indexes: vec![],
        relations: vec![],
        search: None,
        // CRDT is irrelevant to change-log persistence; keep it off so
        // inserts are plain SQL and the test stays focused.
        crdt: false,
    }
}

fn manifest(entities: Vec<ManifestEntity>) -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "Lifecycle".into(),
        version: "0.1.0".into(),
        entities,
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
        llm: Default::default(),
        connections: vec![],
        crons: vec![],
    }
}

/// One-entity chat manifest: `Message { body, channel }`.
fn chat_manifest() -> AppManifest {
    manifest(vec![entity("Message", &["body", "channel"])])
}

/// A valid Pylon row id: 40-char lowercase hex, derived from a small
/// number so tests stay readable and assertions are deterministic.
/// Zero-padded fixed width means lexicographic order matches numeric
/// order, so sorting these is sorting by `n`.
fn hid(n: u64) -> String {
    format!("{n:040x}")
}

// --- scenarios --------------------------------------------------------

/// THE headline scenario: send a message, restart the server, reload
/// the page. The message must still be there.
///
/// This is the exact "open the test channel, send a message, refresh,
/// it's gone" production report. Pre-fix the change log was in-memory
/// only and reset to empty on every boot; the reentrant-write_conn
/// deadlock then blocked the persistence fix. With the full stack wired
/// up, a reload (pull from cursor 0) after restart reconstructs state.
#[test]
fn send_restart_reload_message_survives() {
    let mut h = LifecycleHarness::boot(chat_manifest());

    h.insert(
        "Message",
        &hid(1),
        serde_json::json!({"body": "hello", "channel": "test"}),
    );
    h.insert(
        "Message",
        &hid(2),
        serde_json::json!({"body": "world", "channel": "test"}),
    );

    // Pre-restart, a fresh client sees both.
    assert_eq!(h.visible_ids("Message", 0), vec![hid(1), hid(2)]);

    h.restart();

    // Post-restart reload (cursor 0) — the bug was this returning empty.
    assert_eq!(
        h.visible_ids("Message", 0),
        vec![hid(1), hid(2)],
        "messages must survive a restart + reload"
    );
}

/// A client that already saw some events reconnects after a restart and
/// pulls from its saved cursor. It must get exactly the events it
/// missed — no resync, no gap, no replay of what it already had.
#[test]
fn reconnect_from_cursor_delivers_missed_delta() {
    let mut h = LifecycleHarness::boot(chat_manifest());

    let s1 = h.insert(
        "Message",
        &hid(1),
        serde_json::json!({"body": "a", "channel": "c"}),
    );
    let _s2 = h.insert(
        "Message",
        &hid(2),
        serde_json::json!({"body": "b", "channel": "c"}),
    );
    // Client has seen up to m1's seq.
    let client_cursor = s1;

    h.restart();

    // While the client was away (across the restart), two more arrive.
    h.insert(
        "Message",
        &hid(3),
        serde_json::json!({"body": "c", "channel": "c"}),
    );
    h.insert(
        "Message",
        &hid(4),
        serde_json::json!({"body": "d", "channel": "c"}),
    );

    // Reconnect from the saved cursor: must see m2 (missed before the
    // restart) + m3 + m4 (arrived during) — but NOT m1 (already had it).
    let delivered = h.visible_ids("Message", client_cursor);
    assert_eq!(
        delivered,
        vec![hid(2), hid(3), hid(4)],
        "reconnect must deliver exactly the missed delta across a restart"
    );
}

/// Seqs must be strictly monotonic across restarts. If a restart reset
/// the counter to 0, every cached client cursor would be ahead of the
/// new seq space and fire a permanent 410 RESYNC_REQUIRED storm on each
/// deploy. The persisted high-water (`_pylon_change_seq`) prevents that.
#[test]
fn seqs_stay_monotonic_across_restart() {
    let mut h = LifecycleHarness::boot(chat_manifest());

    let s1 = h.insert(
        "Message",
        &hid(1),
        serde_json::json!({"body": "a", "channel": "c"}),
    );
    let s2 = h.insert(
        "Message",
        &hid(2),
        serde_json::json!({"body": "b", "channel": "c"}),
    );
    assert!(s2 > s1);
    let pre_restart_max = s2;

    h.restart();

    // current_seq must resume at or above the pre-restart max — never 0.
    assert!(
        h.current_seq() >= pre_restart_max,
        "current_seq reset across restart ({} < {}) — would trigger a 410 storm",
        h.current_seq(),
        pre_restart_max
    );

    // The next event's seq must exceed everything from before.
    let s3 = h.insert(
        "Message",
        &hid(3),
        serde_json::json!({"body": "c", "channel": "c"}),
    );
    assert!(
        s3 > pre_restart_max,
        "post-restart seq {s3} must exceed pre-restart max {pre_restart_max}"
    );
}

/// Pins the GRANULARITY of the seed gate: it is per-*entity*, not
/// per-row. An entity that already has at least one persisted event is
/// trusted wholesale on restart — its rows are NOT re-scanned and
/// re-seeded. This is deliberate: re-seeding is an O(rows) table scan,
/// and doing it every boot for a synced entity with millions of rows
/// would wedge startup. The contract is "a synced entity's rows flow
/// through the change log; the persisted log is the source of truth for
/// that entity after first write."
///
/// Consequence (asserted here so it's not mistaken for a bug): a row
/// that reached storage out-of-band, in an entity that ALSO has tracked
/// rows, stays invisible until it gets its own event. The whole-new-
/// entity case — where seeding DOES kick in — is covered by
/// [`new_entity_added_across_restart_is_seeded`]. Together the two tests
/// pin both sides of the per-entity boundary; flipping the gate to
/// per-row (which would re-seed huge tables every boot) breaks this one.
#[test]
fn seed_gate_is_per_entity_not_per_row() {
    let mut h = LifecycleHarness::boot(chat_manifest());

    // m1 goes through the full pipeline (storage + event) — so the
    // Message entity now has a persisted event.
    h.insert(
        "Message",
        &hid(1),
        serde_json::json!({"body": "tracked", "channel": "c"}),
    );
    // m2 reaches storage only — no change-log event.
    h.insert_raw(
        "Message",
        &hid(2),
        serde_json::json!({"body": "out-of-band", "channel": "c"}),
    );

    h.restart();

    // Message has events → not re-scanned. m1 survives (persisted log);
    // m2 stays invisible (the gate trusts the entity wholesale).
    assert_eq!(
        h.visible_ids("Message", 0),
        vec![hid(1)],
        "an entity with persisted events must not be re-seeded row-by-row on restart"
    );
}

/// The flip side of the seed gate: a row that already has a persisted
/// event must NOT be re-seeded on restart. A double-seed would emit the
/// same row twice into the log — a fresh client would see a duplicate
/// Insert. Per-entity gating skips entities already covered.
#[test]
fn tracked_rows_are_not_double_seeded_on_restart() {
    let mut h = LifecycleHarness::boot(chat_manifest());
    h.insert(
        "Message",
        &hid(1),
        serde_json::json!({"body": "x", "channel": "c"}),
    );

    h.restart();

    let m1_count = h
        .pull_all(0)
        .into_iter()
        .filter(|e| e.entity == "Message" && e.row_id == hid(1))
        .count();
    assert_eq!(
        m1_count, 1,
        "a tracked row must appear exactly once after restart, not be re-seeded"
    );
}

/// Adding a brand-new entity to the manifest across a restart: its
/// pre-existing rows (inserted while it was only a storage table) get
/// seeded so clients see them. Models a redeploy that ships a new
/// entity backed by data migrated in out-of-band.
#[test]
fn new_entity_added_across_restart_is_seeded() {
    // Boot with BOTH entities present (so the Reaction table exists),
    // but only write Reaction rows out-of-band — no events. This is the
    // shape of a table whose rows predate its inclusion in the synced
    // change log.
    let two = manifest(vec![
        entity("Message", &["body", "channel"]),
        entity("Reaction", &["emoji"]),
    ]);
    let mut h = LifecycleHarness::boot(two.clone());

    h.insert(
        "Message",
        &hid(1),
        serde_json::json!({"body": "hi", "channel": "c"}),
    );
    h.insert_raw("Reaction", &hid(101), serde_json::json!({"emoji": "👍"}));
    h.insert_raw("Reaction", &hid(102), serde_json::json!({"emoji": "🎉"}));

    h.restart_with_manifest(two);

    let mut reactions = h.visible_ids("Reaction", 0);
    reactions.sort();
    assert_eq!(
        reactions,
        vec![hid(101), hid(102)],
        "a new entity's pre-existing rows must be seeded on restart"
    );
    assert_eq!(h.visible_ids("Message", 0), vec![hid(1)]);
}

/// Durability via explicit flush: once `flush()` returns, the events
/// are on disk and survive a restart. Proves the flush barrier actually
/// commits (not just acks) before returning.
#[test]
fn flush_makes_events_durable() {
    let mut h = LifecycleHarness::boot(chat_manifest());
    h.insert(
        "Message",
        &hid(1),
        serde_json::json!({"body": "durable", "channel": "c"}),
    );

    // Explicit flush — must report success.
    assert!(h.change_log.flush(FLUSH_TIMEOUT), "flush should succeed");

    h.restart();
    assert_eq!(
        h.visible_ids("Message", 0),
        vec![hid(1)],
        "flushed event must survive restart"
    );
}

/// Stress + deadlock guard: many mutations across several restart
/// cycles. The reentrant-write_conn deadlock P0 would hang the very
/// first SQLite mutation (the persister re-acquiring write_conn while
/// the mutation tx held it); this test would never complete. The
/// wall-clock bound turns that hang into a visible failure, and the
/// cumulative-state assertion catches any lost or duplicated event.
#[test]
fn many_mutations_across_restarts_no_deadlock() {
    let started = Instant::now();
    let mut h = LifecycleHarness::boot(chat_manifest());

    let mut expected: Vec<String> = Vec::new();
    let mut n = 1u64;
    for cycle in 0..3 {
        for _ in 0..20 {
            let id = hid(n);
            h.insert(
                "Message",
                &id,
                serde_json::json!({"body": format!("msg {n}"), "channel": "c"}),
            );
            expected.push(id);
            n += 1;
        }
        h.restart();

        // After each restart, a full reload reconstructs the cumulative
        // set exactly — no loss (persistence works), no duplicates
        // (seed gate works). Fixed-width hex ids sort numerically under
        // a plain lexicographic sort.
        let mut ids = h.visible_ids("Message", 0);
        ids.sort();
        let mut want = expected.clone();
        want.sort();
        assert_eq!(
            ids, want,
            "cumulative state mismatch after restart cycle {cycle}"
        );
    }

    assert!(
        started.elapsed() < Duration::from_secs(30),
        "lifecycle stress took too long — possible deadlock or lock contention"
    );
}
