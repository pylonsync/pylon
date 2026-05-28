//! `/api/sync/*` + GDPR `/api/admin/users/:id/{export,purge}` routes.
//!
//! Sync pull filters changes through the read-policy fence so callers
//! can't sidestep entity policies via the change feed (regression
//! coverage in the auth-matrix scaffold).
//!
//! Structure: `handle` is a thin top-level router. It dispatches to:
//!   - `handle_snapshot_pull` — snapshot pagination at `since=0`
//!   - `handle_delta_pull`    — change-log tail at `since>0`
//!   - GDPR export/purge      — admin-gated data-subject endpoints
//!   - `handle_push`          — client write path through the
//!                              shared mutation pipeline
//! Plus `project_change_for_caller` — the read-policy + projection
//! filter shared by both pull paths.

use crate::mutate::{apply_mutation, MutationCtx, MutationError, MutationOp};
use crate::{gdpr_export, gdpr_purge, json_error_safe, require_admin, RouterContext};
use pylon_http::HttpMethod;
use pylon_sync::{ChangeEvent, ChangeKind, SyncCursor};

/// Minimal URL percent-decoder for the snapshot cursor query param.
/// Avoids dragging in a `urlencoding` crate for one usage site.
fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()
}

/// Minimal URL percent-encoder for query params. Encodes anything
/// that isn't an unreserved RFC 3986 character.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    if url.starts_with("/api/sync/pull") && method == HttpMethod::Get {
        return Some(handle_pull(ctx, url));
    }
    if let Some(out) = handle_gdpr(ctx, method, url) {
        return Some(out);
    }
    if url == "/api/sync/push" && method == HttpMethod::Post {
        return Some(handle_push(ctx, body));
    }
    None
}

// ---------------------------------------------------------------------------
// Pull paths
// ---------------------------------------------------------------------------

fn handle_pull(ctx: &RouterContext, url: &str) -> (u16, String) {
    let since: u64 = url
        .split("since=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if since == 0 {
        handle_snapshot_pull(ctx, url)
    } else {
        handle_delta_pull(ctx, since)
    }
}

/// Fresh client (since=0): synthesize a snapshot from current entity
/// rows instead of relying on the in-memory change-log tail. Startup
/// seeds the log with current rows, but retention can evict those
/// seed events before the first client connects; a 50k-row table
/// behind a 10k-event log loses 40k seeds. Snapshot-on-pull closes
/// the gap deterministically: every entity is queried at request
/// time, each row emitted as a synthetic Insert with seq = snapshot
/// as-of seq so the caller's cursor lands at the same position the
/// log tail would have given them.
///
/// Pagination: `?snapshot_after=<urlenc(json)>` carries the entity we
/// paused inside + the last row id emitted for it + the pinned
/// as-of seq, so successive pages share one consistent snapshot
/// frame even if writes land mid-pagination.
fn handle_snapshot_pull(ctx: &RouterContext, url: &str) -> (u16, String) {
    const SNAPSHOT_BATCH_LIMIT: usize = 1000;
    const RAW_FETCH_CHUNK: usize = 200;

    let snapshot_after_raw: Option<String> = url
        .split("snapshot_after=")
        .nth(1)
        .and_then(|s| s.split('&').next())
        .map(|s| s.to_string());
    let snapshot_after_parsed: Option<serde_json::Value> = snapshot_after_raw
        .as_deref()
        .map(url_decode)
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    let snapshot_after: Option<(String, Option<String>)> =
        snapshot_after_parsed.as_ref().and_then(|v| {
            let entity = v.get("e").and_then(|s| s.as_str())?.to_string();
            let after_id = v.get("a").and_then(|s| s.as_str()).map(|s| s.to_string());
            Some((entity, after_id))
        });
    // Pin snapshot_seq across all pages of a single client's snapshot
    // fetch. Without that, each page would capture a fresh
    // `current_seq` and a write landing between pages could be
    // skipped (page 1 emitted row at seq=N, write landed at seq=N+1,
    // final page advanced cursor to N+2 — the in-between write lost
    // because pages claimed seq=N+2 too). The seq is captured once
    // on the first page and carried through `snapshot_after`.
    let snapshot_seq: u64 = snapshot_after_parsed
        .as_ref()
        .and_then(|v| v.get("s").and_then(|s| s.as_u64()))
        .unwrap_or_else(|| ctx.change_log.current_seq());

    let manifest = ctx.store.manifest();
    let auth_user = &manifest.auth.user;
    let mut changes: Vec<ChangeEvent> = Vec::new();
    let mut next_after: Option<(String, Option<String>)> = None;
    let resume_entity = snapshot_after.as_ref().map(|c| c.0.clone());
    let resume_after_id = snapshot_after.as_ref().and_then(|c| c.1.clone());
    let mut started = resume_entity.is_none();

    'outer: for entity in &manifest.entities {
        if entity.name.starts_with('_') {
            continue;
        }
        if !started {
            if Some(&entity.name) == resume_entity.as_ref() {
                started = true;
            } else {
                continue;
            }
        }
        let initial_after = if Some(&entity.name) == resume_entity.as_ref() {
            resume_after_id.clone()
        } else {
            None
        };
        let mut entity_after = initial_after;
        loop {
            let raw =
                match ctx
                    .store
                    .list_after(&entity.name, entity_after.as_deref(), RAW_FETCH_CHUNK)
                {
                    Ok(r) => r,
                    Err(_) => break,
                };
            if raw.is_empty() {
                break;
            }
            let raw_len = raw.len();
            for row in raw {
                let row_id = row
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if row_id.is_empty() {
                    continue;
                }
                entity_after = Some(row_id.clone());
                if !matches!(
                    ctx.policy_engine
                        .check_entity_read(&entity.name, ctx.auth_ctx, Some(&row)),
                    pylon_policy::PolicyResult::Allowed
                ) {
                    continue;
                }
                // Apply both projections in one pass: User entity
                // allowlist + `serverOnly` field strip. Without the
                // second, fields like `Org.stripeCustomerId.serverOnly()`
                // leak through the snapshot path even though
                // `/api/entities` enforces them.
                let projected_data = Some(crate::project_row_for_wire(
                    manifest,
                    auth_user,
                    &entity.name,
                    row,
                ));
                changes.push(ChangeEvent {
                    seq: snapshot_seq,
                    entity: entity.name.clone(),
                    row_id,
                    kind: ChangeKind::Insert,
                    data: projected_data,
                    prev_data: None,
                    timestamp: String::new(),
                });
                if changes.len() >= SNAPSHOT_BATCH_LIMIT {
                    next_after = Some((entity.name.clone(), entity_after.clone()));
                    break 'outer;
                }
            }
            if raw_len < RAW_FETCH_CHUNK {
                break;
            }
        }
    }

    // Stay at `last_seq: 0` while paginating so a follow-up pull with
    // since=0 still routes back to this branch. Only advance to the
    // captured snapshot_seq on the FINAL batch (no more snapshot
    // pages) so the client transitions to the change-log tail at the
    // right "as of" point.
    let has_more = next_after.is_some();
    let cursor_seq = if has_more { 0 } else { snapshot_seq };
    let next_after_str = next_after.map(|(e, a)| {
        let payload = serde_json::json!({"e": e, "a": a, "s": snapshot_seq}).to_string();
        url_encode(&payload)
    });
    let resp = serde_json::json!({
        "changes": changes,
        "cursor": { "last_seq": cursor_seq },
        "has_more": has_more,
        "snapshot_after": next_after_str,
    });
    (200, resp.to_string())
}

/// Catching-up client (since>0): replay the change-log tail since the
/// caller's cursor, with the read-policy fence applied to each event.
fn handle_delta_pull(ctx: &RouterContext, since: u64) -> (u16, String) {
    match ctx.change_log.pull(&SyncCursor { last_seq: since }, 100) {
        Ok(mut resp) => {
            resp.changes = resp
                .changes
                .into_iter()
                .filter_map(|ev| project_change_for_caller(ctx, ev))
                .collect();
            // Wire-level projection on every kept event (data +
            // prev_data) so server-only fields don't leak via the
            // visibility-flip path either.
            let manifest = ctx.store.manifest();
            let auth_user = &manifest.auth.user;
            for ev in resp.changes.iter_mut() {
                if let Some(data) = ev.data.take() {
                    ev.data =
                        Some(crate::project_row_for_wire(manifest, auth_user, &ev.entity, data));
                }
                if let Some(prev) = ev.prev_data.take() {
                    ev.prev_data =
                        Some(crate::project_row_for_wire(manifest, auth_user, &ev.entity, prev));
                }
            }
            (
                200,
                serde_json::to_string(&resp).unwrap_or_else(|_| "{}".into()),
            )
        }
        Err(pylon_sync::PullError::ResyncRequired { oldest_seq, .. }) => (
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
        ),
    }
}

/// Apply the read-policy fence to a single change event. Returns the
/// event the caller should see (post-projection — but projection
/// itself is applied by the caller) or `None` to drop it. Behavior:
///
///   - post allowed → keep as-is (with prev_data stripped)
///   - post denied AND Update AND prev allowed → synthesize a Delete
///     at the same seq so the caller drops the now-invisible row
///   - post denied otherwise → drop silently (callers can't tell a
///     row was filtered from a row that never existed)
fn project_change_for_caller(ctx: &RouterContext, ev: ChangeEvent) -> Option<ChangeEvent> {
    let post_allowed = matches!(
        ctx.policy_engine
            .check_entity_read(&ev.entity, ctx.auth_ctx, ev.data.as_ref()),
        pylon_policy::PolicyResult::Allowed
    );
    if post_allowed {
        // Strip `prev_data` before shipping — it's a server-internal
        // field used only for the dual-check; leaving it on the wire
        // leaks the pre-update row to every recipient.
        return Some(ChangeEvent {
            prev_data: None,
            ..ev
        });
    }
    if matches!(ev.kind, ChangeKind::Update) && ev.prev_data.is_some() {
        let pre_allowed = matches!(
            ctx.policy_engine
                .check_entity_read(&ev.entity, ctx.auth_ctx, ev.prev_data.as_ref()),
            pylon_policy::PolicyResult::Allowed
        );
        if pre_allowed {
            return Some(ChangeEvent {
                seq: ev.seq,
                entity: ev.entity.clone(),
                row_id: ev.row_id.clone(),
                kind: ChangeKind::Delete,
                data: ev.prev_data.clone(),
                prev_data: None,
                timestamp: ev.timestamp.clone(),
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------
// GDPR endpoints
// ---------------------------------------------------------------------------

fn handle_gdpr(ctx: &RouterContext, method: HttpMethod, url: &str) -> Option<(u16, String)> {
    let tail = url.strip_prefix("/api/admin/users/")?;
    let tail = tail.split('?').next().unwrap_or(tail);
    let (user_id, action) = tail.split_once('/')?;
    if user_id.is_empty() {
        return None;
    }
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
    None
}

// ---------------------------------------------------------------------------
// Push path
// ---------------------------------------------------------------------------

/// POST /api/sync/push — client-safe public write path. Each op runs
/// through the same mutation pipeline as the entity-API handlers
/// (apply_mutation), so a non-admin caller's write must pass the
/// action-specific policy check (insert/update/delete) AND any
/// registered before_* plugin hook (validation, tenant-scope, audit).
///
/// Invariant: partial-failure batches return the per-op result
/// envelope intact — successful ops apply and broadcast; failed ops
/// short-circuit with their error code without rolling back siblings.
fn handle_push(ctx: &RouterContext, body: &str) -> (u16, String) {
    let push_req: pylon_sync::PushRequest = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                json_error_safe(
                    "INVALID_JSON",
                    "Invalid request body",
                    &format!("Invalid JSON: {e}"),
                ),
            );
        }
    };

    let mut applied = 0u32;
    let mut errors: Vec<String> = Vec::new();
    let mut deduped = 0u32;
    // Highest seq we assigned, so the response cursor reflects real
    // current_seq instead of `change_log.len()` which silently
    // diverges from truth as retention evicts old events.
    let mut max_seq: u64 = 0;
    // Per-op result envelope: { op_id, row_id, entity, kind, status,
    // seq?, error? }. Clients map each entry back to its op_id (or
    // array index when op_id is absent) to know exactly which
    // mutations applied, were deduplicated, or failed.
    let mut op_results: Vec<serde_json::Value> = Vec::with_capacity(push_req.changes.len());

    for change in &push_req.changes {
        // Tristate op-id state machine:
        //   Proceed       → run the write; on failure forget
        //   InFlight      → concurrent writer mid-flight; respond
        //                   "pending" so the client retries when
        //                   the first writer commits
        //   Replayed{seq} → first write already applied; return
        //                   cached seq so the client's optimistic
        //                   row adopts the canonical seq instead of
        //                   waiting for the WS rebroadcast
        if let Some(ref op_id) = change.op_id {
            match ctx.change_log.claim_op_id(op_id) {
                pylon_sync::OpClaim::Proceed => {}
                pylon_sync::OpClaim::InFlight => {
                    deduped += 1;
                    op_results.push(serde_json::json!({
                        "op_id": op_id,
                        "entity": change.entity,
                        "row_id": change.row_id,
                        "kind": change_kind_label(&change.kind),
                        "status": "pending",
                    }));
                    continue;
                }
                pylon_sync::OpClaim::Replayed { seq } => {
                    deduped += 1;
                    if seq > max_seq {
                        max_seq = seq;
                    }
                    op_results.push(serde_json::json!({
                        "op_id": op_id,
                        "entity": change.entity,
                        "row_id": change.row_id,
                        "kind": change_kind_label(&change.kind),
                        "status": "replayed",
                        "seq": seq,
                    }));
                    continue;
                }
            }
        }
        // Map the wire change kind onto a typed MutationOp so the
        // shared pipeline runs identical policy + plugin + reread +
        // broadcast logic as the entity REST handlers and
        // /api/link/unlink. Returns the assigned seq (Some on apply,
        // None when the reread missed and no event was appended).
        let mctx = MutationCtx::from_router(ctx);
        let op_id_opt = change.op_id.clone();
        let kind_label = change_kind_label(&change.kind);
        let outcome = run_push_op(&mctx, change);

        let result_envelope = |status: &str,
                               row_id: Option<&str>,
                               seq: Option<u64>,
                               error: Option<(String, String)>|
         -> serde_json::Value {
            let mut e = serde_json::json!({
                "op_id": op_id_opt,
                "entity": change.entity,
                "row_id": row_id.unwrap_or(&change.row_id),
                "kind": kind_label,
                "status": status,
            });
            if let Some(s) = seq {
                e["seq"] = serde_json::Value::from(s);
            }
            if let Some((code, message)) = error {
                e["error"] = serde_json::json!({
                    "code": code,
                    "message": message,
                });
            }
            e
        };

        match outcome {
            Ok(out) => {
                if out.seq > max_seq {
                    max_seq = out.seq;
                }
                applied += 1;
                op_results.push(result_envelope(
                    "applied",
                    Some(&out.row_id),
                    Some(out.seq),
                    None,
                ));
                if let Some(ref op_id) = op_id_opt {
                    ctx.change_log.complete_op_id(op_id, out.seq);
                }
            }
            Err(err) => {
                let (_status, code, message) = crate::mutate::error_response(&err);
                let display = format!(
                    "{} {}/{}: {}",
                    kind_label, change.entity, change.row_id, message
                );
                errors.push(display);
                op_results.push(result_envelope("error", None, None, Some((code, message))));
                if let Some(ref op_id) = op_id_opt {
                    ctx.change_log.forget_op_id(op_id);
                }
            }
        }
    }

    // `current_seq()` is the authoritative position even when retention
    // has evicted older entries; `len()` would drift below the real
    // cursor as events scroll off the tail.
    let cursor_seq = if max_seq > 0 {
        max_seq
    } else {
        ctx.change_log.current_seq()
    };
    (
        200,
        serde_json::json!({
            "applied": applied,
            "deduped": deduped,
            "errors": errors,
            // Per-op result envelope. Clients should prefer this over
            // the count fields for status mapping. Count fields are
            // kept for backwards compat with older SDKs.
            "results": op_results,
            "cursor": {"last_seq": cursor_seq}
        })
        .to_string(),
    )
}

fn change_kind_label(kind: &ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Insert => "insert",
        ChangeKind::Update => "update",
        ChangeKind::Delete => "delete",
    }
}

fn run_push_op(
    mctx: &MutationCtx,
    change: &pylon_sync::ClientChange,
) -> Result<crate::mutate::MutationOutcome, MutationError> {
    match change.kind {
        ChangeKind::Insert => match change.data.as_ref() {
            Some(data) => apply_mutation(
                mctx,
                MutationOp::Insert {
                    entity: &change.entity,
                    data,
                },
            ),
            None => Err(MutationError::Store {
                code: "INVALID_INPUT".into(),
                message: "insert requires data".into(),
            }),
        },
        ChangeKind::Update => match change.data.as_ref() {
            Some(data) => apply_mutation(
                mctx,
                MutationOp::Update {
                    entity: &change.entity,
                    row_id: &change.row_id,
                    data,
                },
            ),
            None => Err(MutationError::Store {
                code: "INVALID_INPUT".into(),
                message: "update requires data".into(),
            }),
        },
        ChangeKind::Delete => apply_mutation(
            mctx,
            MutationOp::Delete {
                entity: &change.entity,
                row_id: &change.row_id,
            },
        ),
    }
}
