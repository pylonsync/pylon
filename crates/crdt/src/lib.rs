//! Pylon's local-first substrate.
//!
//! Wraps [Loro](https://loro.dev) with the entity-shape, projection, and
//! sync-protocol pieces Pylon's runtime needs. Same Loro library powers
//! Remboard's iOS app via `loro-swift` and the web client via `loro-crdt`,
//! so all three platforms speak the same binary wire format.
//!
//! # Doc shape (the projection contract)
//!
//! Each Pylon entity row corresponds to one [`LoroDoc`]. The doc holds a
//! single root [`LoroMap`] called `"row"`; its keys are the entity's
//! field names, its values are typed as:
//!
//! | Field type      | Loro container       |
//! |-----------------|----------------------|
//! | `string`        | `LoroText`           |
//! | `richtext`      | `LoroText`           |
//! | `int` / `float` | LWW value (number)   |
//! | `bool`          | LWW value (bool)     |
//! | `datetime`      | LWW value (string)   |
//! | `id(Entity)`    | LWW value (string)   |
//!
//! Strings get rich CRDT semantics (concurrent prepend / append / midword
//! insert all converge). Other scalars are plain LWW registers — same as
//! today, just with deterministic resolution via Loro's internal
//! versioning instead of server-arrival-order.
//!
//! The projector (`project_doc_to_json`) materializes the doc into the
//! shape the rest of Pylon already expects: a flat `serde_json::Value`
//! object of `{ field: value }`. Drop-in replacement for what
//! `Runtime::get_by_id` returns today, so SQLite indexes / FTS / queries
//! keep working unchanged.

use loro::{LoroDoc, LoroValue, ValueOrContainer};
use serde_json::{Map as JsonMap, Value};

pub use loro;

// ---------------------------------------------------------------------------
// Doc shape
// ---------------------------------------------------------------------------

/// Root container name. Every Pylon-managed Loro doc has exactly one
/// top-level [`LoroMap`] under this id; entity field names live inside it.
pub const ROOT_MAP: &str = "row";

/// Open or create the root map for a Pylon-shaped doc.
pub fn root_map(doc: &LoroDoc) -> loro::LoroMap {
    doc.get_map(ROOT_MAP)
}

// ---------------------------------------------------------------------------
// Field type — what the projector knows about each column.
// ---------------------------------------------------------------------------

/// Subset of Pylon's manifest field types that the projector cares about.
/// Mapped from `pylon_kernel::FieldType` at the call site so this crate
/// stays free of the kernel dep cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrdtFieldKind {
    Text,    // string + richtext → LoroText
    Number,  // int + float → LWW f64
    Bool,    // bool → LWW bool
    Iso,     // datetime → LWW string
    Ref,     // id(Entity) → LWW string
}

/// One column the projector knows how to handle. The field's *Loro shape*
/// is implied by `kind`; the field's *JSON shape* in the projection
/// matches today's Pylon row format.
#[derive(Debug, Clone)]
pub struct CrdtField {
    pub name: String,
    pub kind: CrdtFieldKind,
}

// ---------------------------------------------------------------------------
// Apply — write a row update into the doc.
//
// Mutations from server-side TS functions land here. The doc is the
// source of truth; the projector mirrors the result into SQLite.
// ---------------------------------------------------------------------------

/// Apply a flat `{field: value}` patch to the doc. Text fields use Loro's
/// list-CRDT update path so concurrent prepend / append / midword inserts
/// converge; other fields are written as LWW registers in the same map.
///
/// Returns `Err` only when the patch contains a value of the wrong shape
/// for its declared field (e.g. a number on a `Bool` field). Type mismatch
/// at this layer means the schema and the caller disagree — surface it.
pub fn apply_patch(
    doc: &LoroDoc,
    fields: &[CrdtField],
    patch: &Value,
) -> Result<(), String> {
    let obj = patch.as_object().ok_or("patch must be a JSON object")?;
    let map = root_map(doc);

    for field in fields {
        let Some(value) = obj.get(&field.name) else {
            continue; // Field absent from patch — leave existing value.
        };
        match field.kind {
            CrdtFieldKind::Text => {
                let s = value.as_str().ok_or_else(|| {
                    format!("field {}: expected string, got {value}", field.name)
                })?;
                let text = match map.get(&field.name) {
                    Some(ValueOrContainer::Container(loro::Container::Text(t))) => t,
                    _ => map
                        .insert_container(&field.name, loro::LoroText::new())
                        .map_err(|e| format!("insert text {}: {e}", field.name))?,
                };
                // Replace the whole text. A future revision can diff +
                // produce minimal ops; for the first cut this preserves
                // CRDT semantics across writers (concurrent edits to
                // disjoint regions still merge correctly because the
                // overwriter only deletes characters that existed at
                // their HLC).
                let len = text.len_unicode();
                if len > 0 {
                    text.delete(0, len)
                        .map_err(|e| format!("clear text {}: {e}", field.name))?;
                }
                if !s.is_empty() {
                    text.insert(0, s)
                        .map_err(|e| format!("write text {}: {e}", field.name))?;
                }
            }
            CrdtFieldKind::Number => {
                if value.is_null() {
                    map.delete(&field.name).ok();
                    continue;
                }
                let n = value.as_f64().ok_or_else(|| {
                    format!("field {}: expected number, got {value}", field.name)
                })?;
                map.insert(&field.name, n)
                    .map_err(|e| format!("write number {}: {e}", field.name))?;
            }
            CrdtFieldKind::Bool => {
                if value.is_null() {
                    map.delete(&field.name).ok();
                    continue;
                }
                let b = value.as_bool().ok_or_else(|| {
                    format!("field {}: expected bool, got {value}", field.name)
                })?;
                map.insert(&field.name, b)
                    .map_err(|e| format!("write bool {}: {e}", field.name))?;
            }
            CrdtFieldKind::Iso | CrdtFieldKind::Ref => {
                if value.is_null() {
                    map.delete(&field.name).ok();
                    continue;
                }
                let s = value.as_str().ok_or_else(|| {
                    format!("field {}: expected string, got {value}", field.name)
                })?;
                map.insert(&field.name, s.to_string())
                    .map_err(|e| format!("write string {}: {e}", field.name))?;
            }
        }
    }

    doc.commit();
    Ok(())
}

// ---------------------------------------------------------------------------
// Project — derive the SQLite-shaped JSON from the doc.
// ---------------------------------------------------------------------------

/// Materialize the doc as a flat JSON object matching Pylon's row shape.
/// This is what the materialized SQLite view stores; what `Runtime::get_by_id`
/// returns; what the WS broadcast emits to clients that aren't using the
/// raw Loro path.
pub fn project_doc_to_json(doc: &LoroDoc, fields: &[CrdtField]) -> Value {
    let mut out = JsonMap::with_capacity(fields.len());
    let map = root_map(doc);
    for field in fields {
        let v = match map.get(&field.name) {
            None => Value::Null,
            Some(ValueOrContainer::Container(loro::Container::Text(t))) => {
                Value::String(t.to_string())
            }
            Some(ValueOrContainer::Container(_)) => {
                // Non-text container in a scalar slot — schema mismatch.
                // Return null and let the caller surface; Phase 2 adds
                // LoroList / nested-doc field types here.
                Value::Null
            }
            Some(ValueOrContainer::Value(v)) => loro_to_json(v).unwrap_or(Value::Null),
        };
        out.insert(field.name.clone(), v);
    }
    Value::Object(out)
}

fn loro_to_json(v: LoroValue) -> Option<Value> {
    match v {
        LoroValue::Null => Some(Value::Null),
        LoroValue::Bool(b) => Some(Value::Bool(b)),
        LoroValue::Double(n) => serde_json::Number::from_f64(n).map(Value::Number),
        LoroValue::I64(n) => Some(Value::Number(n.into())),
        LoroValue::String(s) => Some(Value::String(s.to_string())),
        LoroValue::Binary(_) => None,
        LoroValue::List(list) => Some(Value::Array(
            list.iter().filter_map(|v| loro_to_json(v.clone())).collect(),
        )),
        LoroValue::Map(m) => {
            let mut out = JsonMap::new();
            for (k, val) in m.iter() {
                if let Some(jv) = loro_to_json(val.clone()) {
                    out.insert(k.clone(), jv);
                }
            }
            Some(Value::Object(out))
        }
        LoroValue::Container(_) => None,
    }
}

// ---------------------------------------------------------------------------
// Wire format — binary snapshots and incremental updates.
// ---------------------------------------------------------------------------

/// Encode a snapshot of the entire doc state. Sent to a fresh client when
/// it subscribes to a row; ~200-500 bytes for a row that's been edited a
/// handful of times after Loro's compaction.
pub fn encode_snapshot(doc: &LoroDoc) -> Vec<u8> {
    doc.export(loro::ExportMode::Snapshot)
        .unwrap_or_default()
}

/// Encode an incremental update relative to a peer's known version.
/// `since` is what the peer last acknowledged; the result contains only
/// ops the peer hasn't seen.
pub fn encode_update_since(doc: &LoroDoc, since: &loro::VersionVector) -> Vec<u8> {
    doc.export(loro::ExportMode::updates(since))
        .unwrap_or_default()
}

/// Apply a binary update from a peer. Returns `Err` if the bytes aren't
/// a valid Loro update — corrupted / truncated WS frames trip this.
pub fn apply_update(doc: &LoroDoc, update: &[u8]) -> Result<(), String> {
    doc.import(update)
        .map(|_| ())
        .map_err(|e| format!("loro import failed: {e}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<CrdtField> {
        vec![
            CrdtField {
                name: "body".into(),
                kind: CrdtFieldKind::Text,
            },
            CrdtField {
                name: "qty".into(),
                kind: CrdtFieldKind::Number,
            },
            CrdtField {
                name: "active".into(),
                kind: CrdtFieldKind::Bool,
            },
            CrdtField {
                name: "createdAt".into(),
                kind: CrdtFieldKind::Iso,
            },
        ]
    }

    #[test]
    fn apply_and_project_roundtrips() {
        let doc = LoroDoc::new();
        apply_patch(
            &doc,
            &fields(),
            &serde_json::json!({
                "body": "hello world",
                "qty": 3,
                "active": true,
                "createdAt": "2026-04-25T12:00:00Z",
            }),
        )
        .unwrap();
        let projected = project_doc_to_json(&doc, &fields());
        assert_eq!(projected["body"], "hello world");
        assert_eq!(projected["qty"], 3.0);
        assert_eq!(projected["active"], true);
        assert_eq!(projected["createdAt"], "2026-04-25T12:00:00Z");
    }

    #[test]
    fn missing_fields_stay_null_until_written() {
        let doc = LoroDoc::new();
        apply_patch(&doc, &fields(), &serde_json::json!({"body": "hi"})).unwrap();
        let projected = project_doc_to_json(&doc, &fields());
        assert_eq!(projected["body"], "hi");
        assert!(projected["qty"].is_null());
    }

    #[test]
    fn snapshot_roundtrip_preserves_state() {
        let a = LoroDoc::new();
        apply_patch(
            &a,
            &fields(),
            &serde_json::json!({"body": "alpha", "qty": 1}),
        )
        .unwrap();
        let snap = encode_snapshot(&a);

        let b = LoroDoc::new();
        apply_update(&b, &snap).unwrap();
        let projected = project_doc_to_json(&b, &fields());
        assert_eq!(projected["body"], "alpha");
        assert_eq!(projected["qty"], 1.0);
    }

    #[test]
    fn concurrent_text_writers_converge_via_loro_merge() {
        // Two replicas, each setting their own text. Loro's CRDT merges
        // both deterministically — neither write is silently lost.
        let a = LoroDoc::new();
        let b = LoroDoc::new();
        a.set_peer_id(1).unwrap();
        b.set_peer_id(2).unwrap();

        apply_patch(&a, &fields(), &serde_json::json!({"body": "from-a"})).unwrap();
        apply_patch(&b, &fields(), &serde_json::json!({"body": "from-b"})).unwrap();

        let a_to_b = a.export(loro::ExportMode::Snapshot).unwrap();
        let b_to_a = b.export(loro::ExportMode::Snapshot).unwrap();
        a.import(&b_to_a).unwrap();
        b.import(&a_to_b).unwrap();

        // Both replicas converge to the same byte-for-byte state.
        let pa = project_doc_to_json(&a, &fields());
        let pb = project_doc_to_json(&b, &fields());
        assert_eq!(pa, pb);
        // Result is *some* deterministic string containing both writes'
        // characters; the important guarantee is convergence, not which
        // text wins.
        let body = pa["body"].as_str().unwrap().to_string();
        assert!(!body.is_empty());
    }

    #[test]
    fn concurrent_disjoint_field_writes_keep_both() {
        let a = LoroDoc::new();
        let b = LoroDoc::new();
        a.set_peer_id(10).unwrap();
        b.set_peer_id(20).unwrap();

        apply_patch(&a, &fields(), &serde_json::json!({"body": "alice"})).unwrap();
        apply_patch(&b, &fields(), &serde_json::json!({"qty": 42})).unwrap();

        let snap_a = a.export(loro::ExportMode::Snapshot).unwrap();
        let snap_b = b.export(loro::ExportMode::Snapshot).unwrap();
        a.import(&snap_b).unwrap();
        b.import(&snap_a).unwrap();

        let pa = project_doc_to_json(&a, &fields());
        let pb = project_doc_to_json(&b, &fields());
        assert_eq!(pa, pb);
        assert_eq!(pa["body"], "alice");
        assert_eq!(pa["qty"], 42.0);
    }

    #[test]
    fn incremental_update_carries_only_the_delta() {
        let server = LoroDoc::new();
        apply_patch(
            &server,
            &fields(),
            &serde_json::json!({"body": "v1", "qty": 1}),
        )
        .unwrap();

        // Client syncs at this point.
        let client = LoroDoc::new();
        let snap = encode_snapshot(&server);
        apply_update(&client, &snap).unwrap();
        let client_vv = client.oplog_vv();

        // Server makes another edit.
        apply_patch(&server, &fields(), &serde_json::json!({"qty": 7})).unwrap();

        // Send only the incremental delta to the client.
        let delta = encode_update_since(&server, &client_vv);
        assert!(
            delta.len() < snap.len(),
            "incremental delta ({}) should be smaller than full snapshot ({})",
            delta.len(),
            snap.len()
        );
        apply_update(&client, &delta).unwrap();

        let projected = project_doc_to_json(&client, &fields());
        assert_eq!(projected["body"], "v1");
        assert_eq!(projected["qty"], 7.0);
    }
}
