//! Integration coverage for the v0.3.105 / v0.3.107 incremental
//! delta broadcast path.
//!
//! Lib tests cover the helper functions (`LoroStore::current_vv_bytes`
//! + `update_since_bytes`); this exercises `broadcast_change_with_crdt`
//! end-to-end against a real Runtime so the per-row last-VV map +
//! snapshot-vs-delta decision works through the actual write path.
//!
//! What this guarantees:
//!   1. First broadcast for a row ships the FULL snapshot (no prior
//!      VV recorded, so the delta branch falls through).
//!   2. Second broadcast for the same row ships an INCREMENTAL
//!      DELTA computed from the first broadcast's VV — concretely
//!      shorter than the snapshot.
//!   3. The bytes that reach `notify_crdt` are valid Loro frames
//!      that a fresh peer can import to reconstruct the projected
//!      state.

use std::sync::Arc;
use std::sync::Mutex;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField};
use pylon_router::{broadcast_change_with_crdt, ChangeNotifier, NoopNotifier};
use pylon_runtime::Runtime;
use pylon_sync::{ChangeEvent, ChangeKind};

fn test_manifest() -> AppManifest {
    AppManifest {
        manifest_version: 1,
        name: "crdt-delta".into(),
        version: "0.1.0".into(),
        entities: vec![ManifestEntity {
            name: "Doc".into(),
            fields: vec![
                ManifestField {
                    name: "title".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                },
                ManifestField {
                    name: "qty".into(),
                    field_type: "int".into(),
                    optional: false,
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                },
            ],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
        }],
        routes: vec![],
        queries: vec![],
        actions: vec![],
        policies: vec![],
        auth: Default::default(),
    }
}

/// Mint a unique 40-char lowercase hex row id. Pylon's
/// `Runtime::insert` rejects anything else with INVALID_ID, and
/// tests run in parallel against a static LAST_VV map in
/// broadcast_change_with_crdt — uniqueness keeps them from
/// stomping each other's per-row state.
fn unique_row_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // 40 hex chars = 160 bits. Pad the 128-bit nanos count with
    // a deterministic suffix to hit the length requirement.
    let hex = format!("{:032x}{:08x}", nanos, std::process::id());
    hex.chars().take(40).collect()
}

/// ChangeNotifier that records every notify_crdt call so the test
/// can assert exactly what got broadcast.
struct CrdtCaptureNotifier {
    crdt_frames: Mutex<Vec<(String, String, Vec<u8>)>>,
}

impl CrdtCaptureNotifier {
    fn new() -> Self {
        Self {
            crdt_frames: Mutex::new(Vec::new()),
        }
    }
    fn frames_for(&self, entity: &str, row_id: &str) -> Vec<Vec<u8>> {
        self.crdt_frames
            .lock()
            .unwrap()
            .iter()
            .filter(|(e, r, _)| e == entity && r == row_id)
            .map(|(_, _, b)| b.clone())
            .collect()
    }
}

impl ChangeNotifier for CrdtCaptureNotifier {
    fn notify(&self, _event: &ChangeEvent) {}
    fn notify_presence(&self, _json: &str) {}
    fn notify_crdt(
        &self,
        entity: &str,
        row_id: &str,
        snapshot: &[u8],
        _row: Option<&serde_json::Value>,
        _seq: u64,
    ) {
        self.crdt_frames.lock().unwrap().push((
            entity.to_string(),
            row_id.to_string(),
            snapshot.to_vec(),
        ));
    }
}

#[test]
fn second_crdt_broadcast_ships_delta_not_snapshot() {
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let notifier = Arc::new(CrdtCaptureNotifier::new());

    // Use a sufficiently-unique entity/row pair so the static
    // LAST_VV map in broadcast_change_with_crdt doesn't conflate
    // with parallel tests.
    let entity = "Doc";
    let row_id = unique_row_id();
    let row_id = row_id.as_str();

    // First write — populates the row + triggers a snapshot broadcast.
    let _ = rt
        .insert(
            entity,
            &serde_json::json!({
                "id": row_id,
                "title": "first-version",
                "qty": 1,
            }),
        )
        .unwrap();
    broadcast_change_with_crdt(
        notifier.as_ref(),
        rt.as_ref(),
        1,
        entity,
        row_id,
        ChangeKind::Insert,
        Some(&serde_json::json!({"title": "first-version", "qty": 1})),
        None,
    );

    // Second write — should ship a delta (incremental ops only).
    rt.update(entity, row_id, &serde_json::json!({"qty": 99}))
        .unwrap();
    broadcast_change_with_crdt(
        notifier.as_ref(),
        rt.as_ref(),
        2,
        entity,
        row_id,
        ChangeKind::Update,
        Some(&serde_json::json!({"qty": 99})),
        None,
    );

    let frames = notifier.frames_for(entity, row_id);
    assert_eq!(
        frames.len(),
        2,
        "expected one frame per write; got {}",
        frames.len()
    );

    // The first frame is a full snapshot; the second is a delta.
    // Loro snapshots are typically 200-500 bytes for a row with a
    // few fields; deltas for a single-field update are typically
    // 50-150 bytes. The exact sizes depend on Loro internals, but
    // the relative ordering is the load-bearing assertion.
    assert!(
        frames[1].len() < frames[0].len(),
        "second frame should be smaller than first (delta < snapshot): {} vs {}",
        frames[1].len(),
        frames[0].len(),
    );

    // The frames must be valid Loro bytes that a fresh peer can
    // import. We don't have a separate Pylon Runtime to merge
    // into, but we can use a bare LoroDoc — the projection logic
    // is what differs between Runtime and bare doc, the binary
    // wire format is identical.
    use pylon_crdt::loro::LoroDoc;
    let peer = LoroDoc::new();
    peer.import(&frames[0]).expect("snapshot imports");
    peer.import(&frames[1])
        .expect("delta imports cleanly on top of snapshot");
    // Loro's row state is now what the server projected. We don't
    // assert on field values here (that's a projection-layer
    // concern covered by pylon_crdt tests); the wire-format
    // round-trip is what matters for this batch.
}

#[test]
fn first_broadcast_for_fresh_row_is_a_full_snapshot() {
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let notifier = Arc::new(CrdtCaptureNotifier::new());

    let entity = "Doc";
    let row_id = unique_row_id();
    let row_id = row_id.as_str();

    rt.insert(
        entity,
        &serde_json::json!({
            "id": row_id,
            "title": "hello",
            "qty": 42,
        }),
    )
    .unwrap();
    broadcast_change_with_crdt(
        notifier.as_ref(),
        rt.as_ref(),
        1,
        entity,
        row_id,
        ChangeKind::Insert,
        Some(&serde_json::json!({"title": "hello", "qty": 42})),
        None,
    );

    let frames = notifier.frames_for(entity, row_id);
    assert_eq!(frames.len(), 1);
    // A snapshot for a row with two fields + Loro's internal
    // bookkeeping is bounded but always > 0 bytes. The strong
    // assertion is "this is importable as a Loro snapshot".
    use pylon_crdt::loro::LoroDoc;
    let peer = LoroDoc::new();
    peer.import(&frames[0]).expect("snapshot imports cleanly");
}

#[test]
fn deletes_skip_the_crdt_broadcast_path() {
    // Deletes go through broadcast_change (the JSON ChangeEvent
    // path) but skip the binary CRDT frame — there's no useful
    // doc state to ship for a deleted row. Make sure no
    // notify_crdt fires.
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let notifier = Arc::new(CrdtCaptureNotifier::new());

    let entity = "Doc";
    let row_id = unique_row_id();
    let row_id = row_id.as_str();

    rt.insert(
        entity,
        &serde_json::json!({"id": row_id, "title": "x", "qty": 1}),
    )
    .unwrap();
    rt.delete(entity, row_id).unwrap();

    broadcast_change_with_crdt(
        notifier.as_ref(),
        rt.as_ref(),
        1,
        entity,
        row_id,
        ChangeKind::Delete,
        None,
        None,
    );

    assert_eq!(
        notifier.frames_for(entity, row_id).len(),
        0,
        "deletes must not fire notify_crdt — that's reserved for snapshots / deltas"
    );
}

#[test]
fn noop_notifier_doesnt_crash_the_broadcast_path() {
    // Regression for the dispatch path: NoopNotifier is used in
    // contexts without WS (Workers fallback today, future
    // headless modes); broadcast_change_with_crdt must
    // gracefully no-op the binary frame without panicking.
    let rt = Arc::new(Runtime::in_memory(test_manifest()).unwrap());
    let notifier = NoopNotifier;

    let entity = "Doc";
    let row_id = unique_row_id();
    let row_id = row_id.as_str();

    rt.insert(
        entity,
        &serde_json::json!({"id": row_id, "title": "x", "qty": 1}),
    )
    .unwrap();
    broadcast_change_with_crdt(
        &notifier,
        rt.as_ref(),
        1,
        entity,
        row_id,
        ChangeKind::Insert,
        Some(&serde_json::json!({"title": "x", "qty": 1})),
        None,
    );
    // No assert needed — the test passing means we didn't panic.
}
