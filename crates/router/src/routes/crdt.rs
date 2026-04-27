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
    Some(
        match ctx.store.crdt_apply_update(entity, row_id, &update_bytes) {
            Ok(snapshot) => {
                ctx.notifier.notify_crdt(entity, row_id, &snapshot);
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
