//! Entity-CRUD routes: cursor pagination, batch (admin), and the main
//! `/api/entities/<entity>[/<id>]` GET/POST/PATCH/DELETE surface.
//!
//! Every path applies the entity's read/insert/update/delete policy
//! before dispatching. Per-row policies on PATCH/DELETE evaluate
//! against the EXISTING row (loaded once here) so a caller can't
//! sidestep ownership rules by omitting the ownership field from
//! their patch.

use crate::{
    broadcast_change, broadcast_change_with_crdt, handle_delete, handle_get, handle_insert,
    handle_list, handle_update, json_error, json_error_safe, json_error_with_hint, require_admin,
    RouterContext,
};
use pylon_http::HttpMethod;
use pylon_sync::ChangeKind;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    // GET /api/entities/<entity>/cursor
    if let Some(rest) = url.strip_prefix("/api/entities/") {
        let rest_no_qs = rest.split('?').next().unwrap_or(rest);
        if let Some(entity_name) = rest_no_qs.strip_suffix("/cursor") {
            if method == HttpMethod::Get {
                if let pylon_policy::PolicyResult::Denied {
                    policy_name,
                    reason,
                } = ctx
                    .policy_engine
                    .check_entity_read(entity_name, ctx.auth_ctx, None)
                {
                    tracing::warn!(
                        "[policy] cursor {entity_name} denied by \"{policy_name}\": {reason}"
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
                let after: Option<&str> = url
                    .split("after=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .filter(|s| !s.is_empty());
                let limit: usize = url
                    .split("limit=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(20)
                    .min(100);

                // Fetch one extra so we can detect has_more even after
                // row-filtering drops some entries.
                return Some(match ctx.store.list_after(entity_name, after, limit + 1) {
                    Ok(rows) => {
                        let filtered: Vec<serde_json::Value> = rows
                            .into_iter()
                            .filter(|row| {
                                matches!(
                                    ctx.policy_engine.check_entity_read(
                                        entity_name,
                                        ctx.auth_ctx,
                                        Some(row),
                                    ),
                                    pylon_policy::PolicyResult::Allowed
                                )
                            })
                            .collect();
                        let has_more = filtered.len() > limit;
                        let page: Vec<serde_json::Value> =
                            filtered.into_iter().take(limit).collect();
                        let next_cursor = page
                            .last()
                            .and_then(|r| r.get("id"))
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        (
                            200,
                            serde_json::json!({
                                "data": page,
                                "next_cursor": next_cursor,
                                "has_more": has_more,
                            })
                            .to_string(),
                        )
                    }
                    Err(e) => (400, json_error(&e.code, &e.message)),
                });
            }
        }
    }

    // POST /api/batch (admin-only; bypasses per-op entity policies)
    if url == "/api/batch" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let batch: serde_json::Value = match serde_json::from_str(body) {
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
        let ops = match batch.get("operations").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => {
                return Some((
                    400,
                    json_error(
                        "MISSING_OPERATIONS",
                        "Request body must contain an \"operations\" array",
                    ),
                ));
            }
        };

        let mut results: Vec<serde_json::Value> = Vec::new();
        let mut succeeded: u32 = 0;
        let mut failed: u32 = 0;

        for op in ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");

            match op_type {
                "insert" => {
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match ctx.store.insert(entity, &data) {
                        Ok(id) => {
                            let seq = ctx.change_log.append(
                                entity,
                                &id,
                                ChangeKind::Insert,
                                Some(data.clone()),
                            );
                            broadcast_change_with_crdt(
                                ctx.notifier,
                                ctx.store,
                                seq,
                                entity,
                                &id,
                                ChangeKind::Insert,
                                Some(&data),
                            );
                            results.push(serde_json::json!({"op": "insert", "id": id, "ok": true}));
                            succeeded += 1;
                        }
                        Err(e) => {
                            results.push(
                                serde_json::json!({"op": "insert", "ok": false, "error": e.message}),
                            );
                            failed += 1;
                        }
                    }
                }
                "update" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match ctx.store.update(entity, id, &data) {
                        Ok(updated) => {
                            if updated {
                                let seq = ctx.change_log.append(
                                    entity,
                                    id,
                                    ChangeKind::Update,
                                    Some(data.clone()),
                                );
                                broadcast_change_with_crdt(
                                    ctx.notifier,
                                    ctx.store,
                                    seq,
                                    entity,
                                    id,
                                    ChangeKind::Update,
                                    Some(&data),
                                );
                            }
                            results.push(serde_json::json!({"op": "update", "id": id, "ok": true}));
                            succeeded += 1;
                        }
                        Err(e) => {
                            results.push(
                                serde_json::json!({"op": "update", "id": id, "ok": false, "error": e.message}),
                            );
                            failed += 1;
                        }
                    }
                }
                "delete" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match ctx.store.delete(entity, id) {
                        Ok(deleted) => {
                            if deleted {
                                let seq =
                                    ctx.change_log.append(entity, id, ChangeKind::Delete, None);
                                broadcast_change(
                                    ctx.notifier,
                                    seq,
                                    entity,
                                    id,
                                    ChangeKind::Delete,
                                    None,
                                );
                            }
                            results.push(serde_json::json!({"op": "delete", "id": id, "ok": true}));
                            succeeded += 1;
                        }
                        Err(e) => {
                            results.push(
                                serde_json::json!({"op": "delete", "id": id, "ok": false, "error": e.message}),
                            );
                            failed += 1;
                        }
                    }
                }
                _ => {
                    results.push(
                        serde_json::json!({"op": op_type, "ok": false, "error": "unknown operation"}),
                    );
                    failed += 1;
                }
            }
        }

        return Some((
            200,
            serde_json::json!({
                "results": results,
                "succeeded": succeeded,
                "failed": failed,
            })
            .to_string(),
        ));
    }

    // /api/entities/<entity>[/<id>] GET/POST/PATCH/DELETE
    if let Some(path) = url.strip_prefix("/api/entities/") {
        let path = path.split('?').next().unwrap_or(path);
        let segments: Vec<&str> = path.splitn(2, '/').collect();
        let entity_name = segments[0];
        let entity_id = segments.get(1).filter(|s| !s.is_empty()).copied();

        // Parse body up-front for POST/PATCH so the policy can see
        // incoming data. Parse errors short-circuit to 400 before the
        // store is touched.
        let parsed_body_for_policy: Option<serde_json::Value> = match method {
            HttpMethod::Post | HttpMethod::Patch if !body.trim().is_empty() => {
                match serde_json::from_str(body) {
                    Ok(v) => Some(v),
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
                }
            }
            _ => None,
        };

        // For PATCH/DELETE, evaluate ownership rules against the
        // EXISTING row, not the incoming patch — so a caller can't
        // bypass `data.authorId == auth.userId` by omitting the
        // ownership field from their PATCH body.
        let existing_row_for_policy: Option<serde_json::Value> = match (method, entity_id) {
            (HttpMethod::Patch, Some(id)) | (HttpMethod::Delete, Some(id)) => {
                ctx.store.get_by_id(entity_name, id).ok().flatten()
            }
            _ => None,
        };

        let policy_check = match method {
            HttpMethod::Get => ctx
                .policy_engine
                .check_entity_read(entity_name, ctx.auth_ctx, None),
            HttpMethod::Post => ctx.policy_engine.check_entity_insert(
                entity_name,
                ctx.auth_ctx,
                parsed_body_for_policy.as_ref(),
            ),
            HttpMethod::Patch => ctx.policy_engine.check_entity_update(
                entity_name,
                ctx.auth_ctx,
                existing_row_for_policy.as_ref(),
            ),
            HttpMethod::Delete => ctx.policy_engine.check_entity_delete(
                entity_name,
                ctx.auth_ctx,
                existing_row_for_policy.as_ref(),
            ),
            _ => pylon_policy::PolicyResult::Allowed,
        };
        if let pylon_policy::PolicyResult::Denied {
            policy_name,
            reason,
        } = policy_check
        {
            tracing::warn!(
                "[policy] {method:?} {entity_name} denied by \"{policy_name}\": {reason}"
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

        return Some(match (method, entity_id) {
            (HttpMethod::Get, None) => handle_list(ctx.store, entity_name, url),
            (HttpMethod::Post, None) => handle_insert(ctx, entity_name, body),
            (HttpMethod::Get, Some(id)) => handle_get(ctx.store, entity_name, id),
            (HttpMethod::Patch, Some(id)) => handle_update(ctx, entity_name, id, body),
            (HttpMethod::Delete, Some(id)) => handle_delete(ctx, entity_name, id),
            _ => (405, json_error("METHOD_NOT_ALLOWED", "Method not allowed")),
        });
    }

    None
}
