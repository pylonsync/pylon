//! `/api/crdt/<entity>/<row_id>` — POST a Loro update, server merges
//! into the row's LoroDoc + re-projects to materialized columns +
//! broadcasts the post-merge snapshot to subscribers.
//!
//! Update-policy gated: the existing row is loaded so policies depending
//! on row data (`data.ownerId == auth.userId`) can evaluate, blocking
//! any session (incl. auto-minted guests) from mutating an addressable
//! CRDT row it doesn't own.

use crate::{
    decode_hex, json_error, json_error_safe, json_error_with_hint, require_auth, RouterContext,
};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    let rest = url.strip_prefix("/api/crdt/")?;
    let rest = rest.split('?').next().unwrap_or(rest);
    if method != HttpMethod::Post {
        return None;
    }
    if let Some(err) = require_auth(ctx) {
        return Some(err);
    }
    let mut parts = rest.splitn(2, '/');
    let entity = parts.next().unwrap_or("");
    let row_id = parts.next().unwrap_or("");
    if entity.is_empty() || row_id.is_empty() {
        return Some((
            400,
            json_error("BAD_PATH", "Expected /api/crdt/<entity>/<row_id>"),
        ));
    }
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return Some((
                400,
                json_error_safe(
                    "INVALID_JSON",
                    "Invalid request body",
                    &format!("Invalid JSON: {e}"),
                ),
            ));
        }
    };
    let hex_str = match parsed.get("update").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => {
            return Some((
                400,
                json_error(
                    "MISSING_UPDATE",
                    "Body must contain `update` (hex-encoded Loro bytes)",
                ),
            ));
        }
    };
    let update_bytes = match decode_hex(hex_str) {
        Some(b) => b,
        None => {
            return Some((
                400,
                json_error(
                    "INVALID_HEX",
                    "`update` must be lowercase hex of even length",
                ),
            ));
        }
    };
    let deps = CrdtApplyDeps {
        store: ctx.store,
        policy_engine: ctx.policy_engine,
        plugin_hooks: ctx.plugin_hooks,
        notifier: ctx.notifier,
        change_log: ctx.change_log,
    };
    Some(
        match apply_crdt_update(&deps, ctx.auth_ctx, entity, row_id, &update_bytes, true) {
            Ok(applied) => {
                if let Some(seq) = applied.change_seq {
                    crate::emit_change_seq_header(ctx, seq);
                }
                (200, serde_json::json!({"ok": true}).to_string())
            }
            Err(e) if e.code == "POLICY_DENIED" => (
                403,
                json_error_with_hint(
                    "POLICY_DENIED",
                    "Access denied by policy",
                    "Check your auth token or the policy rules in your schema",
                ),
            ),
            Err(e) => (e.status, json_error(&e.code, &e.message)),
        },
    )
}

/// What a CRDT merge needs besides the caller: the store, the policy
/// engine, the plugin chain, and where to broadcast.
pub struct CrdtApplyDeps<'a> {
    pub store: &'a dyn pylon_http::DataStore,
    pub policy_engine: &'a pylon_policy::PolicyEngine,
    pub plugin_hooks: &'a dyn crate::PluginHookOps,
    pub notifier: &'a dyn crate::ChangeNotifier,
    pub change_log: &'a pylon_sync::ChangeLog,
}

/// A merged update: the post-merge snapshot, and the sequence number of the
/// JSON change event for the row's projected columns (when the row exists
/// after the merge).
#[derive(Debug)]
pub struct CrdtApplied {
    pub snapshot: Vec<u8>,
    pub change_seq: Option<u64>,
}

/// Why a merge was refused, with the HTTP status the route answers.
#[derive(Debug, Clone, PartialEq)]
pub struct CrdtApplyError {
    pub status: u16,
    pub code: String,
    pub message: String,
}

impl CrdtApplyError {
    fn new(status: u16, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }
}

/// Merge a Loro update into a row's document, the one path every CRDT write
/// takes (`POST /api/crdt/...` and `ctx.crdt.apply` in server functions):
///
/// 1. With `check_policy`, the entity's update policy runs against the stored
///    row (a caller who may not write it never learns whether it exists).
/// 2. The row must exist, and the entity must replicate CRDT history.
/// 3. The plugin chain's `before_update` runs.
/// 4. The store merges. With `check_policy`, the update policy and the
///    readonly fields are checked again against the row after the merge,
///    before anything commits.
/// 5. `after_update` runs, the binary frame goes to the row's subscribers
///    (each re-checked against its read policy), and a JSON change event goes
///    to everyone else.
///
/// `check_policy` is false only for server functions that run with server
/// trust (`ctx.crdt` without strict function policies, or `ctx.crdt.unsafe`),
/// the same rule as `ctx.db`.
pub fn apply_crdt_update(
    deps: &CrdtApplyDeps,
    auth: &pylon_auth::AuthContext,
    entity: &str,
    row_id: &str,
    update: &[u8],
    check_policy: bool,
) -> Result<CrdtApplied, CrdtApplyError> {
    let existing_row = deps.store.get_by_id(entity, row_id).ok().flatten();
    if check_policy {
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = deps
            .policy_engine
            .check_entity_update(entity, auth, existing_row.as_ref(), None)
        {
            tracing::warn!(
                "[policy] crdt push {entity}/{row_id} denied by \"{policy_name}\": {reason}"
            );
            return Err(CrdtApplyError::new(403, "POLICY_DENIED", reason));
        }
    }
    // Row MUST exist before we merge — otherwise CRDT sidecar state
    // accumulates for non-existent materialized rows (divergent +
    // invisible-to-policy). Caught in the 2026-05-10 codex pass-3
    // audit (P1 NEW).
    if existing_row.is_none() {
        return Err(CrdtApplyError::new(
            404,
            "ROW_NOT_FOUND",
            "CRDT update targets a row that doesn't exist",
        ));
    }
    let manifest = deps.store.manifest();
    if deps.store.has_private_crdt_history(entity)
        || !crate::supports_crdt_replication(manifest, &manifest.auth.user, entity)
    {
        return Err(CrdtApplyError::new(
            403,
            "CRDT_REPLICATION_DISABLED",
            "Binary CRDT updates are disabled for this entity. Use the JSON update API.",
        ));
    }
    // Plugin chain: run `before_update` so TenantScopePlugin and any
    // audit_log / validation plugins observe the write. The CRDT
    // wire shape is opaque bytes, so we pass a synthetic data payload
    // marking it as a CRDT delta — plugins that need the actual
    // post-merge row read it from the store after the merge. This
    // closes the gap where TS-mutation `ctx.db.update` got plugin
    // hooks (v0.3.70 fix) but `/api/crdt/<e>/<row>` did not.
    // Caught in the 2026-05-10 codex pass-3 audit (P1 NEW).
    let mut hook_data = serde_json::json!({
        "_pylon_crdt_update": true,
        "_pylon_update_size_bytes": update.len(),
    });
    if let Err((status, code, msg)) =
        deps.plugin_hooks
            .before_update(entity, row_id, &mut hook_data, auth)
    {
        return Err(CrdtApplyError::new(status, &code, msg));
    }
    // The check above only saw the stored row. The merged doc is known
    // only inside the store's transaction, so the store hands its
    // projected columns to this closure before committing. The update
    // rule must also hold for the row after the merge (see
    // `PolicyEngine::check_entity_update`), and a readonly column (owner,
    // tenant) can't change. Either failure rolls the merge back.
    let authorize = |projected: &serde_json::Value| -> Result<(), pylon_http::DataError> {
        if check_policy {
            authorize_crdt_merge(deps, auth, entity, existing_row.as_ref(), projected)
        } else {
            Ok(())
        }
    };
    let snapshot = match deps
        .store
        .crdt_apply_update(entity, row_id, update, &authorize)
    {
        Ok(snapshot) => snapshot,
        Err(e) => {
            if e.code == "POLICY_DENIED" {
                tracing::warn!("[policy] crdt push {entity}/{row_id} denied: {}", e.message);
            }
            let status = match e.code.as_str() {
                "POLICY_DENIED" => 403,
                "READONLY_FIELD" | "FIELD_TOO_LONG" => 400,
                "ENTITY_NOT_FOUND" => 404,
                "NOT_SUPPORTED" => 400,
                "CRDT_DECODE_FAILED" => 400,
                _ => 500,
            };
            return Err(CrdtApplyError::new(status, &e.code, e.message));
        }
    };
    deps.plugin_hooks
        .after_update(entity, row_id, &hook_data, auth);
    // Fetch the post-merge row so the broadcast can run per-
    // subscriber policy re-checks against it. None → entity-
    // level only (graceful degradation for row-level rules).
    let row_after = deps.store.get_by_id(entity, row_id).ok().flatten();
    // Conservative high-water seq for the revocation
    // tombstone (CRDT route doesn't have an
    // event-specific seq in scope yet — the JSON change
    // event below will get its own seq). The client also
    // fires a catch-up pull on receipt, so being
    // slightly behind the JSON event seq is harmless.
    let revocation_seq = deps.change_log.current_seq();
    deps.notifier.notify_crdt(
        entity,
        row_id,
        &snapshot,
        row_after.as_ref(),
        revocation_seq,
    );
    // ALSO emit a JSON change event so non-CRDT subscribers
    // (the regular `db.useQuery` path, `/api/sync/pull`
    // callers) observe the materialized-column projection
    // that `crdt_apply_update` writes. Without this, normal
    // JSON sync clients see only the binary CRDT frame
    // (which they don't decode) and miss the projected
    // fields the SQL row now reflects.
    //
    // Documented exception to the "all writes go through
    // `apply_mutation`" invariant: the CRDT path already
    // ran the policy gate + plugin before_update hook
    // above, and `notify_crdt` did the binary broadcast.
    // What's left is JSON-only notification of the
    // materialized row to non-CRDT subscribers — handled
    // here with the typed `ChangeRecord::Update`.
    let change_seq = row_after.map(|row| {
        let event = deps.change_log.record(
            entity,
            row_id,
            pylon_sync::ChangeRecord::Update {
                row,
                // The pre-merge row, so visibility-flip subscribers on the
                // JSON sync path get a synthesized Delete tombstone instead
                // of being silently dropped.
                prev: existing_row.clone(),
            },
        );
        crate::broadcast_change_event(
            deps.notifier,
            event.seq,
            entity,
            row_id,
            event.kind.clone(),
            event.data.as_ref(),
            event.prev_data.as_ref(),
        );
        event.seq
    });
    Ok(CrdtApplied {
        snapshot,
        change_seq,
    })
}

/// Authorize a CRDT merge against its projected columns before the store
/// commits it. The row after the merge is the stored row with the
/// projected columns written over it.
fn authorize_crdt_merge(
    deps: &CrdtApplyDeps,
    auth: &pylon_auth::AuthContext,
    entity: &str,
    existing: Option<&serde_json::Value>,
    projected: &serde_json::Value,
) -> Result<(), pylon_http::DataError> {
    // Only columns whose value changes count as written. The projection
    // carries every CRDT column, including ones the peer did not touch.
    let changed: serde_json::Map<String, serde_json::Value> = projected
        .as_object()
        .map(|cols| {
            cols.iter()
                .filter(|(k, v)| {
                    k.as_str() != "id"
                        && !existing
                            .and_then(|row| row.get(k.as_str()))
                            .is_some_and(|stored| same_value(stored, v))
                })
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    let patch = serde_json::Value::Object(changed);
    if let Err((code, message)) =
        crate::reject_readonly_payload(deps.store.manifest(), entity, &patch, auth)
    {
        return Err(pylon_http::DataError {
            code: code.into(),
            message,
        });
    }
    match deps
        .policy_engine
        .check_entity_update(entity, auth, existing, Some(&patch))
    {
        pylon_policy::PolicyResult::Allowed => Ok(()),
        pylon_policy::PolicyResult::Denied { reason, .. } => Err(pylon_http::DataError {
            code: "POLICY_DENIED".into(),
            message: reason,
        }),
    }
}

/// Value equality that ignores storage representation: `1` and `1.0` are
/// the same number, and SQLite's `0`/`1` equal `false`/`true`.
fn same_value(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Bool(x), Value::Number(n)) | (Value::Number(n), Value::Bool(x)) => {
            n.as_f64() == Some(if *x { 1.0 } else { 0.0 })
        }
        _ => a == b,
    }
}
