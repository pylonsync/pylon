//! `/api/link` and `/api/unlink` — set / clear a foreign-key relation
//! on a row. Treated as a write (calls `check_entity_write` against
//! the source entity).

use crate::{broadcast_change_with_crdt, json_error, json_error_safe, RouterContext};
use pylon_http::HttpMethod;
use pylon_sync::ChangeKind;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    if url == "/api/link" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
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
        let entity = data.get("entity").and_then(|v| v.as_str()).unwrap_or("");
        let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let relation = data.get("relation").and_then(|v| v.as_str()).unwrap_or("");
        let target_id = data.get("target_id").and_then(|v| v.as_str()).unwrap_or("");

        // A link is a mutation: it sets a foreign key on the source row.
        // Apply the same write policy as PATCH /api/entities/:name/:id —
        // but evaluate it against the EXISTING source row, never the
        // caller's body. Codex P0: pre-fix, `check_entity_write` got
        // `Some(&data)` (the request body), so a caller could include
        // fake `ownerId`/`tenantId` fields that satisfy a row-scoped
        // write policy like `auth.userId == data.ownerId` even though
        // they don't own the actual row. They'd then proceed to
        // mutate someone else's row's FK. The fix: load the source
        // row server-side first and authorize against THAT.
        let source_row = match ctx.store.get_by_id(entity, id) {
            Ok(Some(row)) => row,
            Ok(None) => {
                return Some((
                    404,
                    json_error("NOT_FOUND", &format!("{entity}/{id} not found")),
                ));
            }
            Err(e) => return Some((500, json_error(&e.code, &e.message))),
        };
        let check = ctx
            .policy_engine
            .check_entity_write(entity, ctx.auth_ctx, Some(&source_row));
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = check
        {
            tracing::warn!("[policy] link on {entity} denied by \"{policy_name}\": {reason}");
            return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
        }

        return Some(match ctx.store.link(entity, id, relation, target_id) {
            Ok(true) => {
                // Broadcast the link as a row update so sync subscribers
                // see the FK change in real time. Without this, link
                // mutations were invisible to WS / SSE clients until
                // their next poll (the pre-2026-05-15 framework gap
                // codex audit pass-3 flagged). `Ok(None)` on re-read
                // = concurrent delete; skip the broadcast.
                match ctx.store.get_by_id(entity, id) {
                    Ok(Some(full)) => {
                        let seq = ctx.change_log.append(
                            entity,
                            id,
                            ChangeKind::Update,
                            Some(full.clone()),
                        );
                        broadcast_change_with_crdt(
                            ctx.notifier,
                            ctx.store,
                            seq,
                            entity,
                            id,
                            ChangeKind::Update,
                            Some(&full),
                        );
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "[link] re-read returned None for {entity}/{id} — concurrent delete; skipping broadcast"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[link] re-read failed for {entity}/{id} ({}): broadcasting partial payload",
                            e.message
                        );
                        let payload = serde_json::json!({ relation: target_id });
                        let seq = ctx.change_log.append(
                            entity,
                            id,
                            ChangeKind::Update,
                            Some(payload.clone()),
                        );
                        broadcast_change_with_crdt(
                            ctx.notifier,
                            ctx.store,
                            seq,
                            entity,
                            id,
                            ChangeKind::Update,
                            Some(&payload),
                        );
                    }
                }
                (200, serde_json::json!({"linked": true}).to_string())
            }
            Ok(false) => (404, json_error("NOT_FOUND", "Source entity not found")),
            Err(e) => (400, json_error(&e.code, &e.message)),
        });
    }

    if url == "/api/unlink" && method == HttpMethod::Post {
        let data: serde_json::Value = match serde_json::from_str(body) {
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
        let entity = data.get("entity").and_then(|v| v.as_str()).unwrap_or("");
        let id = data.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let relation = data.get("relation").and_then(|v| v.as_str()).unwrap_or("");

        // Authorize against the existing source row, NOT the caller's
        // body. Same P0 hole as the link path above: row-scoped write
        // predicates evaluated against attacker-controlled body data
        // let any caller unlink someone else's relation.
        let source_row = match ctx.store.get_by_id(entity, id) {
            Ok(Some(row)) => row,
            Ok(None) => {
                return Some((
                    404,
                    json_error("NOT_FOUND", &format!("{entity}/{id} not found")),
                ));
            }
            Err(e) => return Some((500, json_error(&e.code, &e.message))),
        };
        let check = ctx
            .policy_engine
            .check_entity_write(entity, ctx.auth_ctx, Some(&source_row));
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = check
        {
            tracing::warn!("[policy] unlink on {entity} denied by \"{policy_name}\": {reason}");
            return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
        }

        return Some(match ctx.store.unlink(entity, id, relation) {
            Ok(true) => {
                // Same `Ok(None)` skip as the link path above.
                match ctx.store.get_by_id(entity, id) {
                    Ok(Some(full)) => {
                        let seq = ctx.change_log.append(
                            entity,
                            id,
                            ChangeKind::Update,
                            Some(full.clone()),
                        );
                        broadcast_change_with_crdt(
                            ctx.notifier,
                            ctx.store,
                            seq,
                            entity,
                            id,
                            ChangeKind::Update,
                            Some(&full),
                        );
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "[unlink] re-read returned None for {entity}/{id} — concurrent delete; skipping broadcast"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            "[unlink] re-read failed for {entity}/{id} ({}): broadcasting partial payload",
                            e.message
                        );
                        let payload = serde_json::json!({ relation: serde_json::Value::Null });
                        let seq = ctx.change_log.append(
                            entity,
                            id,
                            ChangeKind::Update,
                            Some(payload.clone()),
                        );
                        broadcast_change_with_crdt(
                            ctx.notifier,
                            ctx.store,
                            seq,
                            entity,
                            id,
                            ChangeKind::Update,
                            Some(&payload),
                        );
                    }
                }
                (200, serde_json::json!({"unlinked": true}).to_string())
            }
            Ok(false) => (404, json_error("NOT_FOUND", "Source entity not found")),
            Err(e) => (400, json_error(&e.code, &e.message)),
        });
    }

    None
}
