//! `/api/crdt/<entity>/<row_id>` — POST a Loro update, server merges
//! into the row's LoroDoc + re-projects to materialized columns +
//! broadcasts the post-merge snapshot to subscribers.
//!
//! Update-policy gated: previously any session (incl. guest, which
//! auto-mints without credentials) could mutate any addressable CRDT
//! row. Now the existing row is loaded so policies depending on row
//! data (`data.ownerId == auth.userId`) can evaluate.

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
    // Run the policy check FIRST against whatever existing row state
    // we have (may be None). A deny-by-default policy bounces here
    // with 403 — no further info leaks. Then we verify the row
    // actually exists and 404 if not. This ordering means an
    // unauth/wrong-tenant caller never learns whether the row
    // exists; only callers who pass the policy gate see the
    // ROW_NOT_FOUND signal.
    let existing_row = ctx.store.get_by_id(entity, row_id).ok().flatten();
    if let pylon_policy::PolicyResult::Denied {
        policy_name,
        reason,
    } = ctx
        .policy_engine
        .check_entity_update(entity, ctx.auth_ctx, existing_row.as_ref())
    {
        tracing::warn!(
            "[policy] crdt push {entity}/{row_id} denied by \"{policy_name}\": {reason}"
        );
        return Some((
            403,
            json_error_with_hint(
                "POLICY_DENIED",
                "Access denied by policy",
                "Check your auth token or the policy rules in your schema",
            ),
        ));
    }
    // Row MUST exist before we merge — otherwise CRDT sidecar state
    // accumulates for non-existent materialized rows (divergent +
    // invisible-to-policy). Caught in the 2026-05-10 codex pass-3
    // audit (P1 NEW).
    if existing_row.is_none() {
        return Some((
            404,
            json_error(
                "ROW_NOT_FOUND",
                "CRDT update targets a row that doesn't exist",
            ),
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
        "_pylon_update_size_bytes": update_bytes.len(),
    });
    if let Err((status, code, msg)) =
        ctx.plugin_hooks
            .before_update(entity, row_id, &mut hook_data, ctx.auth_ctx)
    {
        return Some((status, json_error(&code, &msg)));
    }
    Some(
        match ctx.store.crdt_apply_update(entity, row_id, &update_bytes) {
            Ok(snapshot) => {
                ctx.plugin_hooks
                    .after_update(entity, row_id, &hook_data, ctx.auth_ctx);
                // Fetch the post-merge row so the broadcast can run per-
                // subscriber policy re-checks against it. None → entity-
                // level only (graceful degradation for row-level rules).
                let row_for_authz = ctx.store.get_by_id(entity, row_id).ok().flatten();
                ctx.notifier
                    .notify_crdt(entity, row_id, &snapshot, row_for_authz.as_ref());
                // ALSO emit a JSON change event so non-CRDT subscribers
                // (the regular `db.useQuery` path, `/api/sync/pull`
                // callers) observe the materialized-column projection
                // that `crdt_apply_update` writes. Without this, normal
                // JSON sync clients see only the binary CRDT frame
                // (which they don't decode) and miss the projected
                // fields the SQL row now reflects — they'd be stale
                // until reconcile (codex P1).
                if let Ok(Some(row)) = ctx.store.get_by_id(entity, row_id) {
                    let seq = ctx.change_log.append(
                        entity,
                        row_id,
                        pylon_sync::ChangeKind::Update,
                        Some(row.clone()),
                    );
                    crate::emit_change_seq_header(ctx, seq);
                    crate::broadcast_change(
                        ctx.notifier,
                        seq,
                        entity,
                        row_id,
                        pylon_sync::ChangeKind::Update,
                        Some(&row),
                    );
                }
                (200, serde_json::json!({"ok": true}).to_string())
            }
            Err(e) => {
                let status = match e.code.as_str() {
                    "ENTITY_NOT_FOUND" => 404,
                    "NOT_SUPPORTED" => 400,
                    "CRDT_DECODE_FAILED" => 400,
                    _ => 500,
                };
                (status, json_error(&e.code, &e.message))
            }
        },
    )
}
