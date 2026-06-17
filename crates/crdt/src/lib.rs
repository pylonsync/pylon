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
//! field names, its values are typed by [`CrdtFieldKind`].
//!
//! ## Defaults
//!
//! | Manifest field type | Default container |
//! |---------------------|-------------------|
//! | `string`            | LWW string register |
//! | `richtext`          | `LoroText` (collaborative editing) |
//! | `int` / `float`     | LWW number register |
//! | `bool`              | LWW bool register |
//! | `datetime`          | LWW string register |
//! | `id(Entity)`        | LWW string register |
//!
//! Most strings in real apps (email, name, slug, status enum, URL) don't
//! need character-level merge — defaulting to LWW keeps the boring case
//! cheap. Pay LoroText overhead only when you opt in.
//!
//! ## Per-field overrides
//!
//! Set `crdt:` on a manifest field to escape the default:
//!
//! | Annotation       | Container         | Use for |
//! |------------------|-------------------|---------|
//! | `"text"`         | `LoroText`        | Long-form text where collaborative merge matters |
//! | `"counter"`      | `LoroCounter`     | Distributed counters (views, votes, likes) |
//! | `"list"`         | `LoroList`        | Ordered collections |
//! | `"movable-list"` | `LoroMovableList` | Reorderable lists (kanban, prioritized todo) |
//! | `"tree"`         | `LoroTree`        | Hierarchical data (folders, threaded comments) |
//! | `"lww"`          | LWW register      | Explicit (matches default for most types) |
//!
//! ## Wire-shape semantics for the non-LWW kinds
//!
//! Pylon's `ctx.db.update(entity, id, patch)` always passes a flat
//! `{field: value}` object. For the LWW kinds the value IS the new
//! state. For the non-LWW kinds the value is interpreted as follows:
//!
//! - **`counter`** — `value` is a NUMBER interpreted as a DELTA to
//!   apply (`counter.increment(value)`). Negative values decrement.
//!   `null` applies the negation of the writer's locally-observed
//!   counter value (`increment(-current)`); under concurrency this
//!   is NOT a distributed "set to zero" — if a peer's `+2` was
//!   in flight, the merged result is 2, not 0. Counters don't
//!   have a true set operation; clients that need absolute-zero
//!   semantics should coordinate at the application layer.
//!   The projection always returns the current absolute value.
//!   Concurrent increments add correctly via Loro's counter CRDT
//!   (two clients each `+1` from view=5 converge to view=7, not 6).
//!
//! - **`list`** — `value` is a JSON ARRAY interpreted as the target
//!   absolute state. apply_patch clears the existing LoroList and
//!   pushes the new items. Under concurrent writes the result is
//!   NOT strict last-write-wins: each peer's `clear()` only deletes
//!   the items it observed locally, and pushes from peers that
//!   raced the clear can survive and interleave with the new
//!   items. Convergence is preserved (both replicas land on the
//!   same final list) but the "absolute target" intuition breaks
//!   down — the merged result may contain items from BOTH writers'
//!   target arrays. For deterministic per-element semantics expose
//!   a dedicated list-op API (`ctx.db.listPush / listDelete`) — TODO.
//!
//! - **`movable-list`** — same wire shape as `list`. The "movable"
//!   advantage (preserving a peer's intended position through
//!   concurrent inserts) only pays off with an explicit move-op
//!   API; the absolute-array shape collapses to LWW like `list`.
//!
//! - **`tree`** — `value` is a JSON ARRAY of `{id, parent, ...meta}`
//!   nodes where `id` is a stable user-supplied string, `parent` is
//!   another node's id or `null` for roots, and any other keys are
//!   stored as Loro metadata. apply_patch reconciles: existing
//!   nodes (matched by user-supplied id stored in meta) get moved
//!   to their new parent + meta-updated; new nodes get created;
//!   nodes absent from the patch get deleted. TreeIDs persist
//!   across writes so concurrent move-ops actually merge.
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

/// CRDT shape for one column. Computed from the field's manifest type
/// plus the optional per-field `crdt:` annotation via [`field_kind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrdtFieldKind {
    /// LWW string register. Default for `string`, `datetime`, `id(...)`.
    LwwString,
    /// LWW number register. Default for `int`, `float`.
    LwwNumber,
    /// LWW bool register. Default for `bool`.
    LwwBool,
    /// LoroText. Default for `richtext`; opt-in for `string` via
    /// `crdt: "text"`. Concurrent prepend / append / midword inserts all
    /// converge via Loro's text CRDT.
    Text,
    /// LoroCounter. Opt-in for `int`/`float` via `crdt: "counter"`.
    /// Concurrent increments add up instead of one stomping the other.
    /// Patch value = delta to apply; projection = current absolute.
    Counter,
    /// LoroList. Opt-in via `crdt: "list"`. Patch value = absolute
    /// target array; projection = current array. Concurrent absolute
    /// writes converge (deterministic final state across replicas)
    /// but the merged result may contain items from BOTH writers'
    /// target arrays — `clear()` only deletes items the writer saw
    /// locally, so peer pushes can survive and interleave. See the
    /// module-level docs for the longer story.
    List,
    /// LoroMovableList. Opt-in via `crdt: "movable-list"`. Wire shape
    /// identical to List; the move-op advantage of LoroMovableList
    /// requires an explicit move-op API (future).
    MovableList,
    /// LoroTree. Opt-in via `crdt: "tree"`. Patch value = flat array
    /// of `{id, parent, ...meta}` nodes (id is user-supplied,
    /// stable across writes). Projection = same flat-array shape.
    /// Reconciles existing nodes by user-id so concurrent moves
    /// actually merge via Loro's tree CRDT.
    Tree,
}

impl CrdtFieldKind {
    /// Returns true if this kind is implemented in `apply_patch` /
    /// `project_doc_to_json`. Every kind is implemented today; the
    /// helper sticks around for now in case a future kind is added
    /// as a reserved surface before its handler lands.
    pub fn is_implemented(self) -> bool {
        matches!(
            self,
            Self::LwwString
                | Self::LwwNumber
                | Self::LwwBool
                | Self::Text
                | Self::Counter
                | Self::List
                | Self::MovableList
                | Self::Tree
        )
    }
}

/// Resolve a manifest field type + optional `crdt:` annotation into the
/// CRDT shape. `field_type` is the raw type string from the manifest
/// (`"string"`, `"int"`, `"id(User)"`, etc.); `crdt` is the typed
/// annotation from `ManifestField.crdt`.
///
/// Returns `Err` when the annotation isn't valid for the field type
/// (e.g. `crdt: "counter"` on a `bool`). Catches schema mistakes at load
/// time, not at first write. Unknown annotations are now caught at
/// manifest deserialization time (typed enum), so this function never
/// has to handle "unknown variant" cases.
pub fn field_kind(
    field_type: &str,
    crdt: Option<pylon_kernel::CrdtAnnotation>,
) -> Result<CrdtFieldKind, String> {
    use pylon_kernel::CrdtAnnotation;
    let base = base_type(field_type);
    let default = match base {
        "string" | "datetime" => CrdtFieldKind::LwwString,
        "int" | "float" => CrdtFieldKind::LwwNumber,
        "bool" => CrdtFieldKind::LwwBool,
        "richtext" => CrdtFieldKind::Text,
        // `id(EntityName)` — base_type strips the parens; the prefix is "id".
        "id" => CrdtFieldKind::LwwString,
        other => {
            return Err(format!("unknown field type: {other}"));
        }
    };
    let Some(annotation) = crdt else {
        return Ok(default);
    };
    let kind = match annotation {
        CrdtAnnotation::Lww => match default {
            // Already LWW — annotation is just documentation.
            k @ (CrdtFieldKind::LwwString | CrdtFieldKind::LwwNumber | CrdtFieldKind::LwwBool) => k,
            // For richtext, "lww" downgrades to LWW string.
            CrdtFieldKind::Text => CrdtFieldKind::LwwString,
            other => other,
        },
        CrdtAnnotation::Text => {
            if !matches!(base, "string" | "richtext") {
                return Err(format!(
                    "crdt: \"text\" only valid on string/richtext fields, got {base}"
                ));
            }
            CrdtFieldKind::Text
        }
        CrdtAnnotation::Counter => {
            if !matches!(base, "int" | "float") {
                return Err(format!(
                    "crdt: \"counter\" only valid on int/float fields, got {base}"
                ));
            }
            CrdtFieldKind::Counter
        }
        CrdtAnnotation::List => CrdtFieldKind::List,
        CrdtAnnotation::MovableList => CrdtFieldKind::MovableList,
        CrdtAnnotation::Tree => CrdtFieldKind::Tree,
    };
    Ok(kind)
}

fn base_type(field_type: &str) -> &str {
    field_type
        .find('(')
        .map(|idx| &field_type[..idx])
        .unwrap_or(field_type)
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
pub fn apply_patch(doc: &LoroDoc, fields: &[CrdtField], patch: &Value) -> Result<(), String> {
    let obj = patch.as_object().ok_or("patch must be a JSON object")?;
    let map = root_map(doc);

    for field in fields {
        let Some(value) = obj.get(&field.name) else {
            continue; // Field absent from patch — leave existing value.
        };
        match field.kind {
            CrdtFieldKind::Text => {
                let s = value
                    .as_str()
                    .ok_or_else(|| format!("field {}: expected string, got {value}", field.name))?;
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
            CrdtFieldKind::LwwNumber => {
                if value.is_null() {
                    map.delete(&field.name).ok();
                    continue;
                }
                let n = value
                    .as_f64()
                    .ok_or_else(|| format!("field {}: expected number, got {value}", field.name))?;
                map.insert(&field.name, n)
                    .map_err(|e| format!("write number {}: {e}", field.name))?;
            }
            CrdtFieldKind::LwwBool => {
                if value.is_null() {
                    map.delete(&field.name).ok();
                    continue;
                }
                let b = value
                    .as_bool()
                    .ok_or_else(|| format!("field {}: expected bool, got {value}", field.name))?;
                map.insert(&field.name, b)
                    .map_err(|e| format!("write bool {}: {e}", field.name))?;
            }
            CrdtFieldKind::LwwString => {
                if value.is_null() {
                    map.delete(&field.name).ok();
                    continue;
                }
                let s = value
                    .as_str()
                    .ok_or_else(|| format!("field {}: expected string, got {value}", field.name))?;
                map.insert(&field.name, s.to_string())
                    .map_err(|e| format!("write string {}: {e}", field.name))?;
            }
            CrdtFieldKind::Counter => {
                let counter = match map.get(&field.name) {
                    Some(ValueOrContainer::Container(loro::Container::Counter(c))) => c,
                    _ => map
                        .insert_container(&field.name, loro::LoroCounter::new())
                        .map_err(|e| format!("insert counter {}: {e}", field.name))?,
                };
                if value.is_null() {
                    // Reset semantics: compute the offset that brings
                    // the counter back to zero from its current value
                    // (Loro counters have no `set` op). Reads `0`
                    // afterwards. Other writers' deltas still apply on
                    // top, which is the expected counter behavior.
                    let current = counter.get_value();
                    if current != 0.0 {
                        counter
                            .increment(-current)
                            .map_err(|e| format!("reset counter {}: {e}", field.name))?;
                    }
                    continue;
                }
                let delta = value.as_f64().ok_or_else(|| {
                    format!("field {}: counter expected number, got {value}", field.name)
                })?;
                if delta != 0.0 {
                    counter
                        .increment(delta)
                        .map_err(|e| format!("counter {}: {e}", field.name))?;
                }
            }
            CrdtFieldKind::List | CrdtFieldKind::MovableList => {
                let arr = value.as_array().ok_or_else(|| {
                    format!(
                        "field {}: list expected JSON array, got {value}",
                        field.name
                    )
                })?;
                // Common rebuild path so List + MovableList share the
                // delete-all-then-push logic. The only wire-shape
                // difference today is the underlying container type;
                // both treat the patch as an absolute target.
                apply_list_rebuild(&map, &field.name, arr, field.kind)?;
            }
            CrdtFieldKind::Tree => {
                let arr = value.as_array().ok_or_else(|| {
                    format!(
                        "field {}: tree expected JSON array of nodes, got {value}",
                        field.name
                    )
                })?;
                apply_tree_reconcile(&map, &field.name, arr)?;
            }
        }
    }

    doc.commit();
    Ok(())
}

/// List + MovableList apply path. Replaces the container contents with
/// the array `items`. Both Loro list types expose the same `insert /
/// delete / clear / push` surface, but the trait objects don't unify
/// (`LoroList` and `LoroMovableList` are different concrete types), so
/// we pattern-match on `kind` and dispatch. Either way the existing
/// container is reused when present (so its container id persists
/// across writes — Loro's merge math depends on stable container
/// identity).
fn apply_list_rebuild(
    map: &loro::LoroMap,
    name: &str,
    items: &[Value],
    kind: CrdtFieldKind,
) -> Result<(), String> {
    // Pre-validate every item before touching the list. Pre-fix the
    // clear ran first, then push-per-item validated; a deeply nested
    // (json_to_loro depth-bound) or otherwise unsupported item N
    // would error AFTER wiping the first N-1 items, leaving the
    // list mutated despite returning Err. Pre-encode to LoroValue
    // here so the mutation pass below can't fail on shape.
    let encoded: Vec<LoroValue> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            json_to_loro(item)
                .ok_or_else(|| format!("list {name}[{i}]: unsupported JSON shape {item}"))
        })
        .collect::<Result<_, _>>()?;

    match kind {
        CrdtFieldKind::List => {
            let list = match map.get(name) {
                Some(ValueOrContainer::Container(loro::Container::List(l))) => l,
                _ => map
                    .insert_container(name, loro::LoroList::new())
                    .map_err(|e| format!("insert list {name}: {e}"))?,
            };
            list.clear()
                .map_err(|e| format!("clear list {name}: {e}"))?;
            for (i, lv) in encoded.into_iter().enumerate() {
                list.push(lv)
                    .map_err(|e| format!("push list {name}[{i}]: {e}"))?;
            }
        }
        CrdtFieldKind::MovableList => {
            let list = match map.get(name) {
                Some(ValueOrContainer::Container(loro::Container::MovableList(l))) => l,
                _ => map
                    .insert_container(name, loro::LoroMovableList::new())
                    .map_err(|e| format!("insert movable list {name}: {e}"))?,
            };
            list.clear()
                .map_err(|e| format!("clear movable list {name}: {e}"))?;
            for (i, lv) in encoded.into_iter().enumerate() {
                list.push(lv)
                    .map_err(|e| format!("push movable list {name}[{i}]: {e}"))?;
            }
        }
        _ => unreachable!("apply_list_rebuild called with non-list kind"),
    }
    Ok(())
}

/// Reconcile a LoroTree against a flat node array. Each input node
/// must have a stable user-supplied `id` (string); `parent` is another
/// node's id or `null` for a root. Anything else on the node object
/// is stored as Loro metadata on the corresponding TreeID.
///
/// Reconciliation strategy:
///   1. Walk the existing tree and build a `user_id → TreeID` map
///      from the `id` field in each node's metadata.
///   2. For every input node:
///      - If user_id is in the map → move to new parent (if changed),
///        update metadata.
///      - Else → create a new TreeID under (lazy-resolved) parent and
///        record it in the map.
///   3. Any user_id present in the existing tree but absent from the
///      input gets deleted.
///
/// Why this dance: TreeIDs are Loro-internal and rebuilt from scratch
/// every write would lose all CRDT history. Mapping by user-supplied
/// id keeps TreeIDs stable across writes so concurrent move ops from
/// other peers actually merge instead of stomping each other.
fn apply_tree_reconcile(map: &loro::LoroMap, name: &str, nodes: &[Value]) -> Result<(), String> {
    // ---- Validation pass — runs BEFORE any tree mutation ----
    //
    // Pre-fix this function created TreeIDs eagerly, then validated
    // parents in a second pass. A patch with a bad parent reference
    // would error AFTER stamping orphan nodes; the outer `doc.commit()`
    // would then publish those orphans even though apply_patch
    // returned Err. Validating first means a malformed patch
    // produces zero side effects.
    //
    // Pre-flight checks:
    //   - every node is a JSON object with a string `id`
    //   - no duplicate `id` in the input
    //   - every non-null `parent` references an `id` that either
    //     exists in the current tree OR appears elsewhere in the
    //     input (forward references inside the patch are fine)
    //   - every meta value is encodable to a LoroValue (bounded
    //     depth check handled by `json_to_loro`)
    let mut parsed: Vec<(String, Option<String>, &serde_json::Map<String, Value>)> =
        Vec::with_capacity(nodes.len());
    let mut input_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (i, node) in nodes.iter().enumerate() {
        let obj = node
            .as_object()
            .ok_or_else(|| format!("tree {name}[{i}]: node must be a JSON object, got {node}"))?;
        let user_id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("tree {name}[{i}]: node missing required string \"id\""))?
            .to_string();
        if !input_ids.insert(user_id.clone()) {
            return Err(format!("tree {name}: duplicate id {user_id:?} in patch"));
        }
        // `parent` must be null/absent (= root) or a string id. A
        // present-but-wrong-type parent (number, array, object) is
        // a typed schema violation — reject explicitly instead of
        // silently coercing to "root", which would let the patch
        // mutate the tree with a quietly-wrong structural intent.
        let parent_id = match obj.get("parent") {
            None => None,
            Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(other) => {
                return Err(format!(
                    "tree {name}[{i}]: node {user_id:?} parent must be string or null, got {other}"
                ));
            }
        };
        // Reject meta entries that can't be encoded — same DoS-guard
        // path lists hit via json_to_loro's depth limit.
        for (k, v) in obj {
            if k == "id" || k == "parent" {
                continue;
            }
            if json_to_loro(v).is_none() {
                return Err(format!(
                    "tree {name}: meta value for {user_id}.{k} unsupported / too deep"
                ));
            }
        }
        parsed.push((user_id, parent_id, obj));
    }

    // Peek at the existing tree (if any) without creating one — a
    // failing validation must NOT instantiate the slot. We need the
    // existing user_id → TreeID map for parent validation; an absent
    // tree means an empty map (no existing nodes to reference).
    let existing_tree: Option<loro::LoroTree> = match map.get(name) {
        Some(ValueOrContainer::Container(loro::Container::Tree(t))) => Some(t),
        _ => None,
    };

    let mut user_to_tree: std::collections::HashMap<String, loro::TreeID> =
        std::collections::HashMap::new();
    if let Some(tree) = existing_tree.as_ref() {
        for tid in tree.nodes() {
            if let Ok(meta) = tree.get_meta(tid) {
                if let Some(ValueOrContainer::Value(LoroValue::String(s))) = meta.get("id") {
                    user_to_tree.insert(s.to_string(), tid);
                }
            }
        }
    }

    // Validate every non-null parent references a node that will
    // EXIST in the post-apply state. The reconcile semantics treat
    // the input as a full target — any existing-tree node not in
    // `input_ids` gets deleted at the end of the mutation pass. So
    // a parent reference to an existing-tree-only node is a silent
    // bug: the move succeeds momentarily, then Loro's cascade
    // delete wipes the child along with its now-vanished parent.
    // Tightening here surfaces that as an explicit error instead
    // of mysteriously dropping rows.
    //
    // Surviving set = `input_ids`. Roots (parent = null) are always
    // valid.
    for (uid, parent_id, _) in &parsed {
        if let Some(pid) = parent_id {
            if !input_ids.contains(pid) {
                let hint = if user_to_tree.contains_key(pid) {
                    "exists in current tree but is missing from this patch (the reconcile would delete it)"
                } else {
                    "is not present anywhere"
                };
                return Err(format!("tree {name}: node {uid:?} parent={pid:?} {hint}"));
            }
        }
    }

    // Cycle detection. With the tightened parent validation above,
    // every parent reference is to another node in the input — so
    // the cycle walk stays entirely within `input_parent` and we
    // don't need to consult the existing-tree parent chain (any
    // existing-tree-only parent would have already been rejected).
    // Loro's tree CRDT rejects cycle-forming `mov` ops at write
    // time, but catching cycles HERE keeps the no-side-effects-on-
    // error guarantee — the alternative is partial meta writes
    // before the offending mov returns Err.
    let input_parent: std::collections::HashMap<&str, Option<&str>> = parsed
        .iter()
        .map(|(uid, pid, _)| (uid.as_str(), pid.as_deref()))
        .collect();
    for (uid, _, _) in &parsed {
        let start = uid.as_str();
        let mut visited: std::collections::HashSet<&str> = std::collections::HashSet::new();
        visited.insert(start);
        let mut cursor = input_parent.get(start).and_then(|p| *p);
        while let Some(p) = cursor {
            if p == start || !visited.insert(p) {
                return Err(format!(
                    "tree {name}: cycle detected involving node {uid:?}"
                ));
            }
            cursor = match input_parent.get(p) {
                Some(next) => *next,
                None => None,
            };
        }
    }

    // ---- Mutation pass ----
    //
    // Validation passed; safe to instantiate the tree container if
    // it didn't exist already. The empty-input edge case (validated
    // OK, no nodes, no existing tree) skips this — we don't create
    // a tree slot for a patch that has nothing to put in it.
    let tree = match existing_tree {
        Some(t) => t,
        None => {
            if parsed.is_empty() {
                return Ok(());
            }
            map.insert_container(name, loro::LoroTree::new())
                .map_err(|e| format!("insert tree {name}: {e}"))?
        }
    };

    let mut keep: std::collections::HashSet<String> =
        std::collections::HashSet::with_capacity(parsed.len());
    let mut pending_parent: Vec<(loro::TreeID, Option<String>)> = Vec::with_capacity(parsed.len());

    for (user_id, parent_id, obj) in parsed {
        keep.insert(user_id.clone());

        let tid = if let Some(&existing) = user_to_tree.get(&user_id) {
            existing
        } else {
            // Create as a root first; the parent pass below moves to
            // the real parent. Avoids forward-reference issues when
            // the input array isn't topologically sorted.
            let new_tid = tree
                .create(loro::TreeParentId::Root)
                .map_err(|e| format!("tree {name}: create node {user_id}: {e}"))?;
            user_to_tree.insert(user_id.clone(), new_tid);
            new_tid
        };
        let meta = tree
            .get_meta(tid)
            .map_err(|e| format!("tree {name}: meta for {user_id}: {e}"))?;
        meta.insert("id", user_id.clone())
            .map_err(|e| format!("tree {name}: write id meta {user_id}: {e}"))?;
        for (k, v) in obj {
            if k == "id" || k == "parent" {
                continue;
            }
            // Validation pass already proved this encodes — `expect`
            // would also be defensible but `ok_or_else` keeps the
            // failure mode consistent with the rest of the function.
            let lv = json_to_loro(v).ok_or_else(|| {
                format!("tree {name}: meta value for {user_id}.{k} unsupported shape {v}")
            })?;
            meta.insert(k, lv)
                .map_err(|e| format!("tree {name}: write meta {user_id}.{k}: {e}"))?;
        }
        pending_parent.push((tid, parent_id));
    }

    for (tid, parent_user_id) in pending_parent {
        let parent_target = match parent_user_id {
            None => loro::TreeParentId::Root,
            // Validation already proved this lookup succeeds; the
            // unwrap here would only fire on an internal bug.
            Some(pid) => loro::TreeParentId::Node(
                *user_to_tree
                    .get(&pid)
                    .expect("validation guaranteed parent exists"),
            ),
        };
        if tree.parent(tid) != Some(parent_target) {
            tree.mov(tid, parent_target)
                .map_err(|e| format!("tree {name}: move {tid:?}: {e}"))?;
        }
    }

    let to_delete: Vec<loro::TreeID> = user_to_tree
        .iter()
        .filter_map(|(uid, &tid)| if keep.contains(uid) { None } else { Some(tid) })
        .collect();
    for tid in to_delete {
        tree.delete(tid)
            .map_err(|e| format!("tree {name}: delete {tid:?}: {e}"))?;
    }

    Ok(())
}

/// Upper bound on JSON / LoroValue nesting depth for list-item and
/// tree-meta encoding. 32 levels is well past any legitimate Pylon
/// usage (deeply-nested structured config tops out around 5-6) but
/// catches hostile payloads before they exhaust the call stack on
/// the runtime's small-stack reader threads (64KB in production).
///
/// Apps that legitimately need to store deeply-nested structures
/// should model them as separate entities + relations rather than
/// jamming the structure into a single CRDT field.
const MAX_VALUE_NEST_DEPTH: u32 = 32;

/// Convert a serde_json::Value into a LoroValue suitable for a list /
/// tree metadata slot. Returns None for shapes Loro doesn't support
/// directly OR for inputs deeper than `MAX_VALUE_NEST_DEPTH` (the
/// caller surfaces the failure as a typed error). Objects + arrays
/// recurse with a depth counter so a hostile `[[[[...]]]]` payload
/// can't overflow the stack on small-stack worker threads.
fn json_to_loro(v: &Value) -> Option<LoroValue> {
    json_to_loro_inner(v, 0)
}

fn json_to_loro_inner(v: &Value, depth: u32) -> Option<LoroValue> {
    if depth > MAX_VALUE_NEST_DEPTH {
        return None;
    }
    match v {
        Value::Null => Some(LoroValue::Null),
        Value::Bool(b) => Some(LoroValue::Bool(*b)),
        // Preserve integers as I64 instead of collapsing every number to a
        // double. Forcing f64 corrupted integers above 2^53 (9007199254740993
        // → ...992) and turned every int into a float, so a List<int> field's
        // projected shape drifted (1 → 1.0) and churned rowsDiffer. i64 covers
        // any value up to ~9.2e18; a real float or a huge u64 (> i64::MAX) has
        // no lossless int form and keeps the f64 path.
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(LoroValue::I64(i))
            } else {
                n.as_f64().map(LoroValue::Double)
            }
        }
        Value::String(s) => Some(LoroValue::String(s.clone().into())),
        Value::Array(arr) => {
            let mut out: Vec<LoroValue> = Vec::with_capacity(arr.len());
            for item in arr {
                out.push(json_to_loro_inner(item, depth + 1)?);
            }
            Some(LoroValue::List(out.into()))
        }
        Value::Object(obj) => {
            let mut out = std::collections::HashMap::with_capacity(obj.len());
            for (k, val) in obj {
                out.insert(k.clone(), json_to_loro_inner(val, depth + 1)?);
            }
            Some(LoroValue::Map(out.into()))
        }
    }
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
            Some(ValueOrContainer::Container(loro::Container::Counter(c))) => {
                // Counters project as their current absolute value even
                // though writes are delta-shaped. An overflow to ±Inf is
                // clamped to a finite number rather than wiped to null.
                serde_json::Number::from_f64(json_safe_f64(c.get_value()))
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            }
            Some(ValueOrContainer::Container(loro::Container::List(l))) => {
                Value::Array(l.to_vec().into_iter().filter_map(loro_to_json).collect())
            }
            Some(ValueOrContainer::Container(loro::Container::MovableList(l))) => {
                Value::Array(l.to_vec().into_iter().filter_map(loro_to_json).collect())
            }
            Some(ValueOrContainer::Container(loro::Container::Tree(t))) => project_tree(&t),
            Some(ValueOrContainer::Container(_)) => {
                // Map / unknown container in a scalar slot — schema
                // mismatch. Return null; an explicit nested-map field
                // type would land here.
                Value::Null
            }
            Some(ValueOrContainer::Value(v)) => loro_to_json(v).unwrap_or(Value::Null),
        };
        out.insert(field.name.clone(), v);
    }
    Value::Object(out)
}

/// Flatten a LoroTree into `[{id, parent, ...meta}, ...]`. Walks every
/// extant (non-deleted) node, reads its metadata, and synthesizes the
/// `parent` field from the tree relationship (overriding any `parent`
/// the metadata might have — Loro is the source of truth for parents,
/// metadata only holds user-supplied display fields).
///
/// Output order: roots first, depth-first traversal within each root,
/// using Loro's `children(parent)` order. Deterministic enough for
/// snapshot tests and for clients that want to render hierarchy
/// without doing their own topological sort.
fn project_tree(tree: &loro::LoroTree) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut stack: Vec<(loro::TreeID, Option<loro::TreeID>)> = tree
        .roots()
        .into_iter()
        .rev()
        .map(|tid| (tid, None))
        .collect();
    while let Some((tid, parent)) = stack.pop() {
        // Push children in reverse so the depth-first walk visits
        // them left-to-right.
        if let Some(children) = tree.children(loro::TreeParentId::Node(tid)) {
            for c in children.into_iter().rev() {
                stack.push((c, Some(tid)));
            }
        }
        let mut obj = JsonMap::new();
        let mut user_id_for_parent_lookup: Option<String> = None;
        if let Ok(meta) = tree.get_meta(tid) {
            // Walk the meta map's entries. We use get_value() for a
            // single shot rather than re-fetching each key one at a
            // time. The "id" key is stamped by apply_tree_reconcile
            // and represents the user-supplied stable id.
            if let LoroValue::Map(m) = meta.get_value() {
                for (k, v) in m.iter() {
                    if let Some(jv) = loro_to_json(v.clone()) {
                        obj.insert(k.clone(), jv);
                    }
                }
                if let Some(uid) = m.get("id").and_then(|v| match v {
                    LoroValue::String(s) => Some(s.to_string()),
                    _ => None,
                }) {
                    user_id_for_parent_lookup = Some(uid);
                }
            }
        }
        // Override `parent` with the resolved user-id of the tree
        // parent so callers always get the canonical structural
        // value rather than a possibly-stale meta entry.
        let parent_user_id = parent.and_then(|p_tid| {
            tree.get_meta(p_tid).ok().and_then(|m| match m.get("id") {
                Some(ValueOrContainer::Value(LoroValue::String(s))) => Some(s.to_string()),
                _ => None,
            })
        });
        match parent_user_id {
            Some(pid) => obj.insert("parent".into(), Value::String(pid)),
            None => obj.insert("parent".into(), Value::Null),
        };
        // If the meta didn't carry an "id" (shouldn't happen for
        // pylon-managed trees but a defensive path for legacy data),
        // fall back to the TreeID's string form so the row is at
        // least introspectable.
        if !obj.contains_key("id") {
            let fallback = user_id_for_parent_lookup.unwrap_or_else(|| format!("{tid:?}"));
            obj.insert("id".into(), Value::String(fallback));
        }
        out.push(Value::Object(obj));
    }
    Value::Array(out)
}

/// JSON numbers can't be `Infinity`/`NaN`. A non-finite f64 — a Counter that
/// overflowed (sum of deltas → ±Inf), or a `Double` smuggled in via the raw
/// Loro API — would otherwise project to `null` (a silent field wipe) or get
/// dropped from a list (a silent index shift). Clamp to the nearest finite
/// value so the field stays a number and an overflow shows up as a huge
/// magnitude instead of vanishing. `NaN` (which a delta-sum can't reach
/// normally) maps to 0.
fn json_safe_f64(n: f64) -> f64 {
    if n.is_nan() {
        0.0
    } else if n.is_infinite() {
        if n > 0.0 {
            f64::MAX
        } else {
            f64::MIN
        }
    } else {
        n
    }
}

fn loro_to_json(v: LoroValue) -> Option<Value> {
    loro_to_json_inner(v, 0)
}

fn loro_to_json_inner(v: LoroValue, depth: u32) -> Option<Value> {
    // Same depth guard as the encode side. A LoroDoc whose tree
    // somehow grew past 32 levels (via direct Loro API use,
    // bypassing apply_patch's encode-side check) would otherwise
    // crash the projection thread on a deeply-nested read.
    if depth > MAX_VALUE_NEST_DEPTH {
        return None;
    }
    match v {
        LoroValue::Null => Some(Value::Null),
        LoroValue::Bool(b) => Some(Value::Bool(b)),
        LoroValue::Double(n) => {
            // Clamp non-finite (see json_safe_f64) so a stray Inf/NaN doesn't
            // drop the element and silently shift later array indices.
            serde_json::Number::from_f64(json_safe_f64(n)).map(Value::Number)
        }
        LoroValue::I64(n) => Some(Value::Number(n.into())),
        LoroValue::String(s) => Some(Value::String(s.to_string())),
        LoroValue::Binary(_) => None,
        LoroValue::List(list) => Some(Value::Array(
            list.iter()
                .filter_map(|v| loro_to_json_inner(v.clone(), depth + 1))
                .collect(),
        )),
        LoroValue::Map(m) => {
            let mut out = JsonMap::new();
            for (k, val) in m.iter() {
                if let Some(jv) = loro_to_json_inner(val.clone(), depth + 1) {
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
    doc.export(loro::ExportMode::Snapshot).unwrap_or_default()
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
                kind: CrdtFieldKind::LwwNumber,
            },
            CrdtField {
                name: "active".into(),
                kind: CrdtFieldKind::LwwBool,
            },
            CrdtField {
                name: "createdAt".into(),
                kind: CrdtFieldKind::LwwString,
            },
        ]
    }

    use pylon_kernel::CrdtAnnotation;

    #[test]
    fn field_kind_defaults() {
        assert_eq!(
            field_kind("string", None).unwrap(),
            CrdtFieldKind::LwwString
        );
        assert_eq!(field_kind("richtext", None).unwrap(), CrdtFieldKind::Text);
        assert_eq!(field_kind("int", None).unwrap(), CrdtFieldKind::LwwNumber);
        assert_eq!(field_kind("float", None).unwrap(), CrdtFieldKind::LwwNumber);
        assert_eq!(field_kind("bool", None).unwrap(), CrdtFieldKind::LwwBool);
        assert_eq!(
            field_kind("datetime", None).unwrap(),
            CrdtFieldKind::LwwString
        );
        assert_eq!(
            field_kind("id(User)", None).unwrap(),
            CrdtFieldKind::LwwString
        );
    }

    #[test]
    fn field_kind_text_opt_in_upgrades_string() {
        assert_eq!(
            field_kind("string", Some(CrdtAnnotation::Text)).unwrap(),
            CrdtFieldKind::Text
        );
        assert_eq!(
            field_kind("richtext", Some(CrdtAnnotation::Text)).unwrap(),
            CrdtFieldKind::Text
        );
    }

    #[test]
    fn field_kind_lww_downgrades_richtext() {
        assert_eq!(
            field_kind("richtext", Some(CrdtAnnotation::Lww)).unwrap(),
            CrdtFieldKind::LwwString
        );
    }

    #[test]
    fn field_kind_text_rejects_non_string_types() {
        assert!(field_kind("int", Some(CrdtAnnotation::Text)).is_err());
        assert!(field_kind("bool", Some(CrdtAnnotation::Text)).is_err());
    }

    #[test]
    fn field_kind_counter_only_on_numbers() {
        assert_eq!(
            field_kind("int", Some(CrdtAnnotation::Counter)).unwrap(),
            CrdtFieldKind::Counter
        );
        assert_eq!(
            field_kind("float", Some(CrdtAnnotation::Counter)).unwrap(),
            CrdtFieldKind::Counter
        );
        assert!(field_kind("string", Some(CrdtAnnotation::Counter)).is_err());
    }

    /// Unknown annotations are now caught at manifest-deserialize time
    /// (typed enum), not at field_kind. This test asserts the wire format
    /// still rejects bad strings via serde — the surface where typos
    /// actually originate.
    #[test]
    fn unknown_annotation_rejected_at_deserialize() {
        let json = r#"{"crdt": "nonsense"}"#;
        let err: Result<pylon_kernel::ManifestField, _> = serde_json::from_str(
            r#"{"name":"x","type":"string","optional":false,"unique":false,"crdt":"nonsense"}"#,
        );
        assert!(
            err.is_err(),
            "typo in crdt annotation must fail to deserialize"
        );
        // Make sure the valid form still works.
        let ok: pylon_kernel::ManifestField = serde_json::from_str(
            r#"{"name":"x","type":"string","optional":false,"unique":false,"crdt":"text"}"#,
        )
        .unwrap();
        assert_eq!(ok.crdt, Some(CrdtAnnotation::Text));
        let _ = json; // keep `json` referenced so unused-binding lint is happy
    }

    #[test]
    fn is_implemented_covers_every_kind() {
        // Every variant lights up now — the helper exists to flag
        // future reserved-but-not-yet-implemented kinds if they ever
        // get added.
        assert!(CrdtFieldKind::LwwString.is_implemented());
        assert!(CrdtFieldKind::Text.is_implemented());
        assert!(CrdtFieldKind::Counter.is_implemented());
        assert!(CrdtFieldKind::List.is_implemented());
        assert!(CrdtFieldKind::MovableList.is_implemented());
        assert!(CrdtFieldKind::Tree.is_implemented());
    }

    #[test]
    fn counter_treats_value_as_delta() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tally".into(),
            kind: CrdtFieldKind::Counter,
        }];
        apply_patch(&doc, &fields, &serde_json::json!({"tally": 3})).unwrap();
        apply_patch(&doc, &fields, &serde_json::json!({"tally": 5})).unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        // 3 + 5 = 8 — two deltas applied, projection is the absolute.
        assert_eq!(projected["tally"], 8.0);
        // Negative deltas decrement.
        apply_patch(&doc, &fields, &serde_json::json!({"tally": -3})).unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(projected["tally"], 5.0);
    }

    #[test]
    fn counter_null_resets_to_zero() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tally".into(),
            kind: CrdtFieldKind::Counter,
        }];
        apply_patch(&doc, &fields, &serde_json::json!({"tally": 42})).unwrap();
        apply_patch(&doc, &fields, &serde_json::json!({"tally": null})).unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(projected["tally"], 0.0);
    }

    #[test]
    fn counter_concurrent_increments_add() {
        // Two replicas each increment from a shared starting state.
        // Counter CRDT semantics: both deltas land. A LWW field
        // would silently drop one writer's increment here.
        let a = LoroDoc::new();
        let b = LoroDoc::new();
        a.set_peer_id(1).unwrap();
        b.set_peer_id(2).unwrap();
        let fields = vec![CrdtField {
            name: "votes".into(),
            kind: CrdtFieldKind::Counter,
        }];
        apply_patch(&a, &fields, &serde_json::json!({"votes": 5})).unwrap();
        let snap = encode_snapshot(&a);
        apply_update(&b, &snap).unwrap();
        // Both peers see 5, both add 1.
        apply_patch(&a, &fields, &serde_json::json!({"votes": 1})).unwrap();
        apply_patch(&b, &fields, &serde_json::json!({"votes": 1})).unwrap();
        // Exchange.
        let a_to_b = a.export(loro::ExportMode::Snapshot).unwrap();
        let b_to_a = b.export(loro::ExportMode::Snapshot).unwrap();
        a.import(&b_to_a).unwrap();
        b.import(&a_to_b).unwrap();
        let pa = project_doc_to_json(&a, &fields);
        let pb = project_doc_to_json(&b, &fields);
        // 5 + 1 + 1 = 7. LWW would have given 6.
        assert_eq!(pa["votes"], 7.0);
        assert_eq!(pa, pb);
    }

    #[test]
    fn list_round_trips_array() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tags".into(),
            kind: CrdtFieldKind::List,
        }];
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tags": ["alpha", "beta", "gamma"]}),
        )
        .unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(
            projected["tags"],
            serde_json::json!(["alpha", "beta", "gamma"])
        );
    }

    #[test]
    fn list_replace_overwrites_previous_contents() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tags".into(),
            kind: CrdtFieldKind::List,
        }];
        apply_patch(&doc, &fields, &serde_json::json!({"tags": ["a", "b"]})).unwrap();
        apply_patch(&doc, &fields, &serde_json::json!({"tags": ["c"]})).unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(projected["tags"], serde_json::json!(["c"]));
    }

    #[test]
    fn movable_list_round_trips_array() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "lanes".into(),
            kind: CrdtFieldKind::MovableList,
        }];
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"lanes": ["todo", "doing", "done"]}),
        )
        .unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(
            projected["lanes"],
            serde_json::json!(["todo", "doing", "done"])
        );
    }

    #[test]
    fn tree_creates_nodes_with_parents() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "comments".into(),
            kind: CrdtFieldKind::Tree,
        }];
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"comments": [
                {"id": "c1", "parent": null, "text": "hello"},
                {"id": "c2", "parent": "c1", "text": "reply"},
            ]}),
        )
        .unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        let arr = projected["comments"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        // Roots-first depth-first: c1 then c2.
        assert_eq!(arr[0]["id"], "c1");
        assert_eq!(arr[0]["parent"], Value::Null);
        assert_eq!(arr[0]["text"], "hello");
        assert_eq!(arr[1]["id"], "c2");
        assert_eq!(arr[1]["parent"], "c1");
        assert_eq!(arr[1]["text"], "reply");
    }

    #[test]
    fn tree_reconcile_updates_parent_on_existing_node() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": null},
                {"id": "b", "parent": "a"},
                {"id": "c", "parent": "a"},
            ]}),
        )
        .unwrap();
        // Move c under b.
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": null},
                {"id": "b", "parent": "a"},
                {"id": "c", "parent": "b"},
            ]}),
        )
        .unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        let arr = projected["tree"].as_array().unwrap();
        let c = arr.iter().find(|n| n["id"] == "c").unwrap();
        assert_eq!(c["parent"], "b");
    }

    #[test]
    fn tree_reconcile_deletes_absent_nodes() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a"}, {"id": "b"}, {"id": "c"}
            ]}),
        )
        .unwrap();
        // Drop b.
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a"}, {"id": "c"}
            ]}),
        )
        .unwrap();
        let projected = project_doc_to_json(&doc, &fields);
        let arr = projected["tree"].as_array().unwrap();
        let ids: Vec<&str> = arr.iter().map(|n| n["id"].as_str().unwrap()).collect();
        assert!(!ids.contains(&"b"));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn tree_unknown_parent_id_rejected() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        let err = apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": "ghost"}
            ]}),
        )
        .expect_err("unknown parent must error");
        assert!(err.contains("not present anywhere"), "got {err}");
    }

    #[test]
    fn tree_validation_failure_leaves_no_orphan_nodes() {
        // Regression: pre-fix, the reconcile created TreeIDs in a
        // first pass and validated parents in a second. A patch with
        // a bad parent would error AFTER the create, so the outer
        // doc.commit would publish orphan root nodes. After the
        // pre-validation refactor, a failed patch produces ZERO
        // visible tree state.
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        // Seed a clean tree.
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [{"id": "root", "parent": null}]}),
        )
        .unwrap();
        // Bad patch: includes "root" + a ghost-parent child.
        let _ = apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "root", "parent": null},
                {"id": "orphan", "parent": "ghost"},
            ]}),
        )
        .expect_err("must error on ghost parent");
        // Verify the doc still has exactly one node (root) — no
        // partially-committed orphan stamping.
        let projected = project_doc_to_json(&doc, &fields);
        let arr = projected["tree"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "failed patch must not create nodes");
        assert_eq!(arr[0]["id"], "root");
    }

    #[test]
    fn tree_duplicate_id_in_patch_rejected() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        let err = apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "x"}, {"id": "x"}
            ]}),
        )
        .expect_err("duplicate id must error");
        assert!(err.contains("duplicate"), "got {err}");
    }

    #[test]
    fn list_rejects_deeply_nested_payload() {
        // 50 levels of nested object — well past MAX_VALUE_NEST_DEPTH.
        // apply_patch must error rather than recurse and crash the
        // small-stack worker thread.
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "items".into(),
            kind: CrdtFieldKind::List,
        }];
        let mut v: serde_json::Value = serde_json::json!({"leaf": 1});
        for _ in 0..50 {
            v = serde_json::json!({"nested": v});
        }
        let patch = serde_json::json!({"items": [v]});
        let err = apply_patch(&doc, &fields, &patch).expect_err("must reject deep nesting");
        assert!(
            err.contains("unsupported JSON shape") || err.contains("too deep"),
            "got {err}"
        );
    }

    #[test]
    fn list_failure_does_not_partial_write() {
        // Seed a list, then issue a patch where the LAST item is
        // too-deep. The pre-validation pass must catch this before
        // clearing the existing list — pre-fix, the clear ran first
        // and the existing items got wiped despite the error.
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tags".into(),
            kind: CrdtFieldKind::List,
        }];
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tags": ["alpha", "beta"]}),
        )
        .unwrap();
        // Build a 50-deep payload as the second item.
        let mut deep: serde_json::Value = serde_json::json!({"leaf": 1});
        for _ in 0..50 {
            deep = serde_json::json!({"nested": deep});
        }
        let _ = apply_patch(&doc, &fields, &serde_json::json!({"tags": ["new1", deep]}))
            .expect_err("must reject deep nesting before any mutation");
        let projected = project_doc_to_json(&doc, &fields);
        // Existing items preserved.
        assert_eq!(projected["tags"], serde_json::json!(["alpha", "beta"]));
    }

    #[test]
    fn tree_self_parent_rejected_pre_mutation() {
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        let err = apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": "a"}
            ]}),
        )
        .expect_err("self-parent must error");
        assert!(err.contains("cycle"), "got {err}");
        // No container created.
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(projected["tree"], Value::Null);
    }

    #[test]
    fn tree_partial_patch_referencing_omitted_parent_rejected() {
        // Seed: a (root), b (under a). A partial patch `{tree: [{id: a,
        // parent: b}]}` omits b — the reconcile semantics would
        // delete b at the end of the mutation pass, leaving a's new
        // parent reference dangling (and Loro's cascade delete would
        // wipe a too). Validation catches this with a clear "parent
        // exists in current tree but is missing from this patch"
        // error rather than silently dropping rows.
        //
        // This also subsumes the "cycle through existing parent"
        // case codex flagged: a→b plus b's existing →a would form a
        // cycle if both moves landed, but the parent-not-in-input
        // check fires first and prevents the entire scenario.
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": null},
                {"id": "b", "parent": "a"},
            ]}),
        )
        .unwrap();
        let before = project_doc_to_json(&doc, &fields);
        let err = apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": "b"}
            ]}),
        )
        .expect_err("partial patch with omitted-parent reference must error");
        assert!(
            err.contains("missing from this patch") || err.contains("would delete"),
            "got {err}"
        );
        // Tree state must be unchanged.
        let after = project_doc_to_json(&doc, &fields);
        assert_eq!(before, after);
    }

    #[test]
    fn tree_cycle_rejected_pre_mutation() {
        // a → b → a chain inside the input. The validation pass
        // catches this; pre-fix the mutation pass would create both
        // nodes + stamp meta before tree.mov rejected the second
        // move, leaving partially-applied state in the doc.
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        let err = apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": "b"},
                {"id": "b", "parent": "a"},
            ]}),
        )
        .expect_err("cycle must error");
        assert!(err.contains("cycle"), "got {err}");
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(projected["tree"], Value::Null);
    }

    #[test]
    fn tree_wrong_typed_parent_rejected() {
        // `parent: 123` must not coerce to root — silent coercion
        // would let a typed-violation patch mutate the tree with a
        // structural intent the caller didn't actually express.
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        let err = apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": 123}
            ]}),
        )
        .expect_err("non-string non-null parent must error");
        assert!(err.contains("parent must be"), "got {err}");
        // And the typed-failure path also leaves no container behind.
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(projected["tree"], Value::Null);
    }

    #[test]
    fn tree_failure_does_not_create_container() {
        // A failed first-write to a tree field must NOT leave an
        // empty tree container in the doc — pre-fix the container
        // got instantiated speculatively before parent validation.
        let doc = LoroDoc::new();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        let _ = apply_patch(
            &doc,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "a", "parent": "ghost"}
            ]}),
        )
        .expect_err("ghost parent must error");
        // Verify the projection sees null for the tree field, meaning
        // no container was created. (An empty container would still
        // project as `[]`, an array, not null.)
        let projected = project_doc_to_json(&doc, &fields);
        assert_eq!(projected["tree"], Value::Null);
    }

    #[test]
    fn tree_concurrent_moves_merge_via_loro() {
        // Stable user_id → TreeID mapping is the whole point: it
        // lets two peers move different nodes concurrently and the
        // merge actually preserves both moves.
        let a = LoroDoc::new();
        let b = LoroDoc::new();
        a.set_peer_id(1).unwrap();
        b.set_peer_id(2).unwrap();
        let fields = vec![CrdtField {
            name: "tree".into(),
            kind: CrdtFieldKind::Tree,
        }];
        apply_patch(
            &a,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "root", "parent": null},
                {"id": "leaf1", "parent": "root"},
                {"id": "leaf2", "parent": "root"},
            ]}),
        )
        .unwrap();
        apply_update(&b, &encode_snapshot(&a)).unwrap();
        // a moves leaf1 to itself. b deletes leaf2.
        apply_patch(
            &a,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "root", "parent": null},
                {"id": "leaf1", "parent": "root"},
                {"id": "leaf2", "parent": "leaf1"},
            ]}),
        )
        .unwrap();
        apply_patch(
            &b,
            &fields,
            &serde_json::json!({"tree": [
                {"id": "root", "parent": null},
                {"id": "leaf1", "parent": "root"},
            ]}),
        )
        .unwrap();
        // Exchange.
        let a_to_b = a.export(loro::ExportMode::Snapshot).unwrap();
        let b_to_a = b.export(loro::ExportMode::Snapshot).unwrap();
        a.import(&b_to_a).unwrap();
        b.import(&a_to_b).unwrap();
        let pa = project_doc_to_json(&a, &fields);
        let pb = project_doc_to_json(&b, &fields);
        // Convergence is the load-bearing assertion.
        assert_eq!(pa, pb);
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

    // ---- #342: List integer precision + Counter overflow -----------------

    fn list_fields() -> Vec<CrdtField> {
        vec![CrdtField {
            name: "data".into(),
            kind: CrdtFieldKind::List,
        }]
    }

    #[test]
    fn list_numbers_keep_integer_shape_and_precision() {
        // Pre-fix every number was forced to f64: integers above 2^53 were
        // corrupted (9007199254740993 → ...992) and every int became a float
        // (1 → 1.0), churning rowsDiffer on List fields.
        let doc = LoroDoc::new();
        apply_patch(
            &doc,
            &list_fields(),
            &serde_json::json!({ "data": [1, 2.5, 9007199254740993_i64] }),
        )
        .unwrap();
        let projected = project_doc_to_json(&doc, &list_fields());
        let arr = projected["data"].as_array().expect("data is an array");

        // Integer stays an integer (not 1.0) — the shape clients diff on.
        assert!(
            arr[0].is_i64(),
            "small int should stay an integer, got {}",
            arr[0]
        );
        assert_eq!(arr[0].as_i64(), Some(1));
        // Float stays a float.
        assert_eq!(arr[1].as_f64(), Some(2.5));
        // Big int survives with full precision (the f64 path returned
        // 9007199254740992).
        assert_eq!(arr[2].as_i64(), Some(9007199254740993));

        // Whole-value round-trip is byte-identical.
        assert_eq!(
            projected["data"],
            serde_json::json!([1, 2.5, 9007199254740993_i64])
        );
    }

    fn counter_fields() -> Vec<CrdtField> {
        vec![CrdtField {
            name: "score".into(),
            kind: CrdtFieldKind::Counter,
        }]
    }

    #[test]
    fn counter_overflow_clamps_to_finite_not_null() {
        // A counter is the sum of its deltas; two near-max increments overflow
        // it to +Inf. Pre-fix the projection ran Number::from_f64(Inf) → None
        // → null, silently wiping the field. It must clamp to a finite number
        // instead — the field stays present and visibly huge.
        let doc = LoroDoc::new();
        apply_patch(
            &doc,
            &counter_fields(),
            &serde_json::json!({ "score": f64::MAX }),
        )
        .unwrap();
        apply_patch(
            &doc,
            &counter_fields(),
            &serde_json::json!({ "score": f64::MAX }),
        )
        .unwrap();

        let projected = project_doc_to_json(&doc, &counter_fields());
        assert!(
            !projected["score"].is_null(),
            "overflowed counter must not project to null"
        );
        let v = projected["score"].as_f64().expect("score is a number");
        assert!(v.is_finite(), "projected counter must be finite, got {v}");
        assert!(v > 0.0);
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
