//! `/api/sync/*` + GDPR `/api/admin/users/:id/{export,purge}` routes.
//!
//! Sync pull filters changes through the read-policy fence so callers
//! can't sidestep entity policies via the change feed (regression
//! coverage in the auth-matrix scaffold).

use crate::{gdpr_export, gdpr_purge, json_error_safe, require_admin, RouterContext};
use pylon_http::HttpMethod;
use pylon_sync::{ChangeKind, SyncCursor};

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    // GET /api/sync/pull
    if url.starts_with("/api/sync/pull") && method == HttpMethod::Get {
        let since: u64 = url
            .split("since=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        match ctx.change_log.pull(&SyncCursor { last_seq: since }, 100) {
            Ok(mut resp) => {
                // Filter changes through the read-policy fence. Previously a
                // caller could pull every mutation regardless of which entities
                // their policy permitted — a silent bypass of read gates.
                resp.changes.retain(|ev| {
                    matches!(
                        ctx.policy_engine.check_entity_read(
                            &ev.entity,
                            ctx.auth_ctx,
                            ev.data.as_ref()
                        ),
                        pylon_policy::PolicyResult::Allowed
                    )
                });
                return Some((
                    200,
                    serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
                ));
            }
            Err(pylon_sync::PullError::ResyncRequired { oldest_seq, .. }) => {
                return Some((
                    410,
                    serde_json::json!({
                        "error": {
                            "code": "RESYNC_REQUIRED",
                            "message": format!(
                                "cursor last_seq={since} is older than the oldest retained seq={oldest_seq}; client must re-sync"
                            ),
                            "oldest_seq": oldest_seq,
                        }
                    })
                    .to_string(),
                ));
            }
        }
    }

    // GDPR data-subject endpoints (admin-gated): export + purge.
    if let Some(tail) = url.strip_prefix("/api/admin/users/") {
        let tail = tail.split('?').next().unwrap_or(tail);
        if let Some((user_id, action)) = tail.split_once('/') {
            if !user_id.is_empty() {
                if action == "export" && method == HttpMethod::Post {
                    if let Some(err) = require_admin(ctx) {
                        return Some(err);
                    }
                    return Some(gdpr_export(ctx, user_id));
                }
                if action == "purge" && method == HttpMethod::Delete {
                    if let Some(err) = require_admin(ctx) {
                        return Some(err);
                    }
                    return Some(gdpr_purge(ctx, user_id));
                }
            }
        }
    }

    // POST /api/sync/push (admin-only)
    if url == "/api/sync/push" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let push_req: pylon_sync::PushRequest = match serde_json::from_str(body) {
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

        let mut applied = 0u32;
        let mut errors: Vec<String> = Vec::new();
        let mut deduped = 0u32;

        for change in &push_req.changes {
            // Idempotency: skip if op_id already processed. Makes
            // at-least-once delivery from the client safe.
            if let Some(ref op_id) = change.op_id {
                if ctx.change_log.has_seen_op_id(op_id) {
                    deduped += 1;
                    continue;
                }
            }
            match change.kind {
                ChangeKind::Insert => {
                    if let Some(ref data) = change.data {
                        match ctx.store.insert(&change.entity, data) {
                            Ok(id) => {
                                ctx.change_log.append(
                                    &change.entity,
                                    &id,
                                    ChangeKind::Insert,
                                    change.data.clone(),
                                );
                                applied += 1;
                            }
                            Err(e) => {
                                errors.push(format!("insert {}: {}", change.entity, e.message))
                            }
                        }
                    }
                }
                ChangeKind::Update => {
                    if let Some(ref data) = change.data {
                        match ctx.store.update(&change.entity, &change.row_id, data) {
                            Ok(_) => {
                                ctx.change_log.append(
                                    &change.entity,
                                    &change.row_id,
                                    ChangeKind::Update,
                                    change.data.clone(),
                                );
                                applied += 1;
                            }
                            Err(e) => errors.push(format!(
                                "update {}/{}: {}",
                                change.entity, change.row_id, e.message
                            )),
                        }
                    }
                }
                ChangeKind::Delete => match ctx.store.delete(&change.entity, &change.row_id) {
                    Ok(_) => {
                        ctx.change_log.append(
                            &change.entity,
                            &change.row_id,
                            ChangeKind::Delete,
                            None,
                        );
                        applied += 1;
                    }
                    Err(e) => errors.push(format!(
                        "delete {}/{}: {}",
                        change.entity, change.row_id, e.message
                    )),
                },
            }
        }

        // Register processed op_ids only for changes that didn't error.
        // Failed applies must NOT be marked seen or a retry is falsely
        // treated as a replay and skipped forever.
        for change in &push_req.changes {
            if let Some(ref op_id) = change.op_id {
                let mention = format!(" {}", change.row_id);
                if !errors
                    .iter()
                    .any(|e| e.contains(&change.entity) && e.contains(&mention))
                {
                    ctx.change_log.remember_op_id(op_id);
                }
            }
        }

        return Some((
            200,
            serde_json::json!({
                "applied": applied,
                "deduped": deduped,
                "errors": errors,
                "cursor": {"last_seq": ctx.change_log.len()}
            })
            .to_string(),
        ));
    }

    None
}
