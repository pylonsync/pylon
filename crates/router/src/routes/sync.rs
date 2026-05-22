//! `/api/sync/*` + GDPR `/api/admin/users/:id/{export,purge}` routes.
//!
//! Sync pull filters changes through the read-policy fence so callers
//! can't sidestep entity policies via the change feed (regression
//! coverage in the auth-matrix scaffold).

use crate::{
    broadcast_change, gdpr_export, gdpr_purge, json_error_safe, require_admin, RouterContext,
};
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
        // Fresh client (since=0): synthesize a snapshot from current
        // entity rows instead of relying on the in-memory change-log
        // tail. Startup seeds the log with current rows, but retention
        // can evict those seed events before the first client connects;
        // a 50k-row table behind a 10k-event log loses 40k seeds. The
        // change-log path's comment hand-waves about "re-seeds on
        // demand" but no such method existed (codex P2). Snapshot-on-
        // pull closes the gap deterministically: every entity is
        // queried at request time, each row emitted as a synthetic
        // Insert with seq = current_seq so the caller's cursor lands
        // at the same position the log tail would have given them.
        //
        // Per-entity cap (10_000 rows) is a footgun guard for very
        // large tables — apps with more rows than that should layer
        // entity-specific queries on top instead of relying on the
        // open firehose. Has-more semantics could paginate this in a
        // future revision; today: ship the snapshot, document the cap.
        if since == 0 {
            const SNAPSHOT_PER_ENTITY_CAP: usize = 10_000;
            let current_seq = ctx.change_log.current_seq();
            let manifest = ctx.store.manifest();
            let auth_user = &manifest.auth.user;
            let mut changes: Vec<pylon_sync::ChangeEvent> = Vec::new();
            for entity in &manifest.entities {
                // Pylon-internal scaffolding entities (names starting
                // with `_`) shouldn't appear in the client snapshot —
                // they're framework-owned and the policy gate would
                // reject them anyway.
                if entity.name.starts_with('_') {
                    continue;
                }
                let rows = match ctx.store.list(&entity.name) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                for row in rows.into_iter().take(SNAPSHOT_PER_ENTITY_CAP) {
                    let row_id = row
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    if row_id.is_empty() {
                        continue;
                    }
                    // Apply the same read-policy fence as the change-log
                    // path so a permissive replica doesn't leak rows the
                    // caller's auth can't see.
                    if !matches!(
                        ctx.policy_engine
                            .check_entity_read(&entity.name, ctx.auth_ctx, Some(&row),),
                        pylon_policy::PolicyResult::Allowed
                    ) {
                        continue;
                    }
                    let projected_data = if entity.name == auth_user.entity {
                        Some(crate::maybe_project_user_row(&entity.name, row, auth_user))
                    } else {
                        Some(row)
                    };
                    changes.push(pylon_sync::ChangeEvent {
                        seq: current_seq,
                        entity: entity.name.clone(),
                        row_id,
                        kind: ChangeKind::Insert,
                        data: projected_data,
                        timestamp: String::new(),
                    });
                }
            }
            let resp = pylon_sync::PullResponse {
                changes,
                cursor: SyncCursor {
                    last_seq: current_seq,
                },
                has_more: false,
            };
            return Some((
                200,
                serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
            ));
        }
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
                // Field-level redaction for User rows. Without this a
                // permissive User read policy (needed for cross-user
                // displayName lookups in chat-style apps) leaks
                // `passwordHash` through the change feed even though
                // `/api/auth/session` strips it.
                let auth_user = &ctx.store.manifest().auth.user;
                for ev in resp.changes.iter_mut() {
                    if ev.entity == auth_user.entity {
                        if let Some(data) = ev.data.take() {
                            ev.data =
                                Some(crate::maybe_project_user_row(&ev.entity, data, auth_user));
                        }
                    }
                }
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
        // Track the highest seq we assigned so the response cursor is
        // the real current_seq, not `change_log.len()` which silently
        // diverges from truth as retention evicts old events.
        let mut max_seq: u64 = 0;

        // Track which op_ids we successfully claimed AND applied, so
        // we know which to keep cached vs roll back if a downstream
        // write fails.
        let mut claimed_op_ids: Vec<String> = Vec::new();
        let mut errored_op_ids: Vec<String> = Vec::new();
        for change in &push_req.changes {
            // Atomic check-and-claim: if another concurrent push raced
            // us to the same op_id, claim_op_id returns false and we
            // dedupe. The previous (has_seen_op_id → apply → remember)
            // sequence had a race window where both racers passed the
            // has-seen check, both applied, and the row got
            // double-mutated. Codex P1.
            if let Some(ref op_id) = change.op_id {
                if !ctx.change_log.claim_op_id(op_id) {
                    deduped += 1;
                    continue;
                }
                claimed_op_ids.push(op_id.clone());
            }
            let mut op_errored = false;
            match change.kind {
                ChangeKind::Insert => {
                    if let Some(ref data) = change.data {
                        match ctx.store.insert(&change.entity, data) {
                            Ok(id) => {
                                let seq = ctx.change_log.append(
                                    &change.entity,
                                    &id,
                                    ChangeKind::Insert,
                                    change.data.clone(),
                                );
                                broadcast_change(
                                    ctx.notifier,
                                    seq,
                                    &change.entity,
                                    &id,
                                    ChangeKind::Insert,
                                    change.data.as_ref(),
                                );
                                if seq > max_seq {
                                    max_seq = seq;
                                }
                                applied += 1;
                            }
                            Err(e) => {
                                op_errored = true;
                                errors.push(format!("insert {}: {}", change.entity, e.message))
                            }
                        }
                    }
                }
                ChangeKind::Update => {
                    if let Some(ref data) = change.data {
                        match ctx.store.update(&change.entity, &change.row_id, data) {
                            Ok(_) => {
                                let seq = ctx.change_log.append(
                                    &change.entity,
                                    &change.row_id,
                                    ChangeKind::Update,
                                    change.data.clone(),
                                );
                                broadcast_change(
                                    ctx.notifier,
                                    seq,
                                    &change.entity,
                                    &change.row_id,
                                    ChangeKind::Update,
                                    change.data.as_ref(),
                                );
                                if seq > max_seq {
                                    max_seq = seq;
                                }
                                applied += 1;
                            }
                            Err(e) => {
                                op_errored = true;
                                errors.push(format!(
                                    "update {}/{}: {}",
                                    change.entity, change.row_id, e.message
                                ));
                            }
                        }
                    }
                }
                ChangeKind::Delete => {
                    // Pre-delete snapshot so the row-scoped read policy
                    // fence on the broadcast path can evaluate against
                    // actual data (see TxStore::delete for the longer
                    // explanation of the ghost-row class).
                    let snapshot = ctx
                        .store
                        .get_by_id(&change.entity, &change.row_id)
                        .ok()
                        .flatten();
                    match ctx.store.delete(&change.entity, &change.row_id) {
                        Ok(_) => {
                            let seq = ctx.change_log.append(
                                &change.entity,
                                &change.row_id,
                                ChangeKind::Delete,
                                snapshot.clone(),
                            );
                            broadcast_change(
                                ctx.notifier,
                                seq,
                                &change.entity,
                                &change.row_id,
                                ChangeKind::Delete,
                                snapshot.as_ref(),
                            );
                            if seq > max_seq {
                                max_seq = seq;
                            }
                            applied += 1;
                        }
                        Err(e) => {
                            op_errored = true;
                            errors.push(format!(
                                "delete {}/{}: {}",
                                change.entity, change.row_id, e.message
                            ));
                        }
                    }
                }
            }
            // Roll back the op_id claim on failure so the client's
            // retry can succeed. Previously failed inserts were
            // un-deduped via string-matching on the row_id in the
            // error message — but the insert error path didn't
            // include the row_id at all, so the claim got silently
            // kept and the retry was dropped. Codex P1.
            if op_errored {
                if let Some(ref op_id) = change.op_id {
                    ctx.change_log.forget_op_id(op_id);
                    errored_op_ids.push(op_id.clone());
                }
            }
        }
        // `claimed_op_ids` was populated at claim time; the loop above
        // already called `forget_op_id` for any op that errored, so the
        // remaining claims correctly reflect successful applies. No
        // further bookkeeping needed — drop the previous string-match
        // post-pass.
        let _ = (&claimed_op_ids, &errored_op_ids);

        // `current_seq()` is the authoritative position even when
        // retention has evicted older entries; `len()` would drift
        // below the real cursor as events scroll off the tail.
        let cursor_seq = if max_seq > 0 {
            max_seq
        } else {
            ctx.change_log.current_seq()
        };
        return Some((
            200,
            serde_json::json!({
                "applied": applied,
                "deduped": deduped,
                "errors": errors,
                "cursor": {"last_seq": cursor_seq}
            })
            .to_string(),
        ));
    }

    None
}
