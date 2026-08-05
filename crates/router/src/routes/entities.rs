//! Entity-CRUD routes: cursor pagination, batch (admin), and the main
//! `/api/entities/<entity>[/<id>]` GET/POST/PATCH/DELETE surface.
//!
//! Every path applies the entity's read/insert/update/delete policy
//! before dispatching. Per-row policies on PATCH/DELETE evaluate
//! against the EXISTING row (loaded once here) so a caller can't
//! sidestep ownership rules by omitting the ownership field from
//! their patch.

use crate::mutate::{apply_mutation, MutationCtx, MutationOp};
use crate::{
    handle_delete, handle_get, handle_insert, handle_list, handle_update, json_error,
    json_error_safe, json_error_with_hint, require_admin, RouterContext,
};
use pylon_http::HttpMethod;

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
                // Per-row read-policy evaluation below; a tenant gate asks the
                // same `exists(...)` question of every row. Memoized for this
                // page only — see ExistsMemo. Same reasoning as the snapshot
                // path in routes/sync.rs.
                let _exists_memo = pylon_policy::ExistsMemo::scope();
                // Codex P1: same underscore-prefix gate as below —
                // framework-managed entities are admin-only on the
                // entity REST surface.
                if entity_name.starts_with('_') && !ctx.auth_ctx.is_admin {
                    return Some((404, json_error("NOT_FOUND", "Entity not found")));
                }
                // Scan-aware pre-check: hard-deny policies (`"false"`
                // or default-deny) still 403 here. Data-dependent
                // predicates (anything referencing `data.X`) are
                // deferred to per-row filtering below — without this
                // deferral, `auth.tenantId == data.orgId` evaluates
                // `data.orgId` to Null at pre-check time, the
                // comparison resolves to false, and every tenant-
                // scoped cursor returns 403 to legitimate members.
                if let pylon_policy::PolicyResult::Denied {
                    policy_name,
                    reason,
                } = ctx
                    .policy_engine
                    .check_entity_scan(entity_name, ctx.auth_ctx)
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
                // `?sync=1` marks this as a REPLICATION fetch — the sync
                // engine filling a client's local replica, not an app reading
                // the table. Only then does the entity's `sync_scope` apply.
                //
                // The distinction matters: `sync: false` documents that direct
                // reads and policies are unchanged, and a scope is the same
                // kind of promise. An app that deliberately reads outside the
                // replicated window (an archive view, an admin report) must
                // still get its rows, so scoping every caller would silently
                // break reads that have nothing to do with replication.
                let replication_fetch = url
                    .split("sync=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .is_some_and(|v| v == "1" || v == "true");
                // Replication fetches page at 1000 (matching the sync pull
                // batch limits) — a reconcile sweep pays one round trip per
                // page, so the cap directly divides sweep latency. Total scan
                // work is the same either way; only the slicing changes. App
                // reads keep the 100 cap.
                let limit_cap: usize = if replication_fetch { 1000 } else { 100 };
                let limit: usize = url
                    .split("limit=")
                    .nth(1)
                    .and_then(|s| s.split('&').next())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(20)
                    .min(limit_cap);

                // Scan raw pages until we accumulate enough visible rows
                // after the read-policy filter or exhaust the source.
                // Previously this fetched `limit + 1` raw rows once and
                // declared `has_more` from the filtered count — codex
                // P1: if hidden rows dominate a page, the caller saw an
                // early `has_more: false` even though more visible rows
                // existed further down the source order. The internal
                // scan cursor advances by the last RAW id processed so
                // we don't re-fetch the same hidden tail; the external
                // `next_cursor` returned to the client is the last
                // visible id (the contract callers depend on).
                const RAW_BATCH: usize = 200;
                let manifest = ctx.store.manifest();
                let auth_user = &manifest.auth.user;
                let mut visible: Vec<serde_json::Value> = Vec::new();
                let mut current_after: Option<String> = after.map(String::from);
                let mut source_exhausted = false;
                let mut fetch_error: Option<pylon_http::DataError> = None;
                while visible.len() <= limit && !source_exhausted {
                    let raw =
                        match ctx
                            .store
                            .list_after(entity_name, current_after.as_deref(), RAW_BATCH)
                        {
                            Ok(r) => r,
                            Err(e) => {
                                fetch_error = Some(e);
                                break;
                            }
                        };
                    if raw.len() < RAW_BATCH {
                        source_exhausted = true;
                    }
                    for row in raw {
                        let row_id = row.get("id").and_then(|v| v.as_str()).map(String::from);
                        // Always advance the internal scan cursor — even
                        // for filtered-out rows, so the NEXT raw fetch
                        // starts strictly past everything we've seen.
                        if row_id.is_some() {
                            current_after = row_id;
                        }
                        let allowed = matches!(
                            ctx.policy_engine.check_entity_read(
                                entity_name,
                                ctx.auth_ctx,
                                Some(&row),
                            ),
                            pylon_policy::PolicyResult::Allowed
                        );
                        if !allowed {
                            continue;
                        }
                        // Replication scope, on top of the read policy and
                        // only for a replication fetch. Same layering as the
                        // snapshot path in routes/sync.rs.
                        if replication_fetch
                            && !matches!(
                                ctx.policy_engine.check_sync_scope(
                                    entity_name,
                                    ctx.auth_ctx,
                                    Some(&row)
                                ),
                                pylon_policy::PolicyResult::Allowed
                            )
                        {
                            continue;
                        }
                        // Apply both wire projections: User-entity
                        // allowlist + `serverOnly` field strip.
                        // Pre-fix only the User-entity allowlist
                        // ran here so server-only fields on non-User
                        // entities leaked through cursor pagination.
                        // Replication fetches additionally shed
                        // `syncOmit` columns — these rows land in the
                        // client replica; a direct read keeps them.
                        visible.push(if replication_fetch {
                            crate::project_row_for_replication(
                                manifest,
                                auth_user,
                                entity_name,
                                row,
                            )
                        } else {
                            crate::project_row_for_wire(manifest, auth_user, entity_name, row)
                        });
                        if visible.len() > limit {
                            break;
                        }
                    }
                }
                if let Some(e) = fetch_error {
                    return Some((400, json_error(&e.code, &e.message)));
                }
                let has_more = visible.len() > limit;
                let page: Vec<serde_json::Value> = visible.into_iter().take(limit).collect();
                let next_cursor = page
                    .last()
                    .and_then(|r| r.get("id"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                return Some((
                    200,
                    serde_json::json!({
                        "data": page,
                        "next_cursor": next_cursor,
                        "has_more": has_more,
                    })
                    .to_string(),
                ));
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

        let mctx = MutationCtx::from_router_admin(ctx);
        for op in ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");

            match op_type {
                "insert" => {
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match apply_mutation(
                        &mctx,
                        MutationOp::Insert {
                            entity,
                            data: &data,
                        },
                    ) {
                        Ok(outcome) => {
                            results.push(serde_json::json!({
                                "op": "insert",
                                "id": outcome.row_id,
                                "ok": true,
                            }));
                            succeeded += 1;
                        }
                        Err(err) => {
                            let (_status, _code, message) = crate::mutate::error_response(&err);
                            results.push(serde_json::json!({
                                "op": "insert",
                                "ok": false,
                                "error": message,
                            }));
                            failed += 1;
                        }
                    }
                }
                "update" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let data = op.get("data").cloned().unwrap_or(serde_json::json!({}));
                    match apply_mutation(
                        &mctx,
                        MutationOp::Update {
                            entity,
                            row_id: id,
                            data: &data,
                        },
                    ) {
                        Ok(_) => {
                            results.push(serde_json::json!({
                                "op": "update",
                                "id": id,
                                "ok": true,
                            }));
                            succeeded += 1;
                        }
                        Err(err) => {
                            let (_status, _code, message) = crate::mutate::error_response(&err);
                            results.push(serde_json::json!({
                                "op": "update",
                                "id": id,
                                "ok": false,
                                "error": message,
                            }));
                            failed += 1;
                        }
                    }
                }
                "delete" => {
                    let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match apply_mutation(&mctx, MutationOp::Delete { entity, row_id: id }) {
                        Ok(_) => {
                            results.push(serde_json::json!({"op": "delete", "id": id, "ok": true}));
                            succeeded += 1;
                        }
                        Err(err) => {
                            let (_status, _code, message) = crate::mutate::error_response(&err);
                            results.push(serde_json::json!({
                                "op": "delete",
                                "id": id,
                                "ok": false,
                                "error": message,
                            }));
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

        // Codex P1: underscore-prefix entities are framework-managed
        // (`_Connection`, `_PylonJobs`, etc.). The policy layer
        // bypasses them as "internal scaffolding" — but the entity
        // REST surface still mounts them, so a SELECT cursor
        // enumerates every user's connection metadata. Gate the
        // surface to admin-only at the route edge.
        if entity_name.starts_with('_') && !ctx.auth_ctx.is_admin {
            return Some((404, json_error("NOT_FOUND", "Entity not found")));
        }

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

        // Pre-fetch the row for GET/PATCH/DELETE-by-id so policies
        // that reference `data.*` (ownership, tenant, etc.) actually
        // see the row they're authorizing against. Skipping this on
        // GET caused `auth.userId == data.id` to ALWAYS deny — the
        // policy evaluator can't compare userId against an absent
        // `data.id`. PATCH/DELETE already loaded the row to defend
        // against bypass-via-omitted-fields; GET joins them.
        let existing_row_for_policy: Option<serde_json::Value> = match (method, entity_id) {
            (HttpMethod::Get, Some(id))
            | (HttpMethod::Patch, Some(id))
            | (HttpMethod::Delete, Some(id)) => ctx.store.get_by_id(entity_name, id).ok().flatten(),
            _ => None,
        };

        // Reject readonly-field writes BEFORE policy evaluation —
        // closes the IDOR-via-update-payload shape (attacker rewrites
        // `authorId` / `orgId` in the PATCH payload to flip
        // ownership). Insert is allowed because readonly means
        // "settable on creation, immutable after." Admin bypasses
        // so migrations + ops scripts can still rewrite. See
        // `reject_readonly_payload` for the field-level semantics.
        if method == HttpMethod::Patch {
            if let Some(payload) = parsed_body_for_policy.as_ref() {
                if let Err((code, message)) = crate::reject_readonly_payload(
                    ctx.store.manifest(),
                    entity_name,
                    payload,
                    ctx.auth_ctx,
                ) {
                    return Some((400, crate::json_error(code, &message)));
                }
            }
        }

        let policy_check = match method {
            // GET-by-id authorizes against the fetched row. GET-list has no
            // row at the edge, so a data-dependent read policy can't be
            // evaluated here — use the scan-level gate (rejects only
            // data-INDEPENDENT denials) and defer per-row filtering to
            // `handle_list`. Without the scan gate + per-row filter, a
            // data-independent-OR policy (`auth.userId != null || …`) passes
            // the edge with data:None and `handle_list` dumps EVERY row
            // (cross-tenant read); and a purely data-dependent policy 403s
            // the whole list for legitimate members.
            HttpMethod::Get if entity_id.is_none() => ctx
                .policy_engine
                .check_entity_scan(entity_name, ctx.auth_ctx),
            HttpMethod::Get => ctx.policy_engine.check_entity_read(
                entity_name,
                ctx.auth_ctx,
                existing_row_for_policy.as_ref(),
            ),
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
            (HttpMethod::Get, None) => handle_list(ctx, entity_name, url),
            (HttpMethod::Post, None) => handle_insert(ctx, entity_name, body),
            (HttpMethod::Get, Some(id)) => handle_get(ctx.store, entity_name, id),
            (HttpMethod::Patch, Some(id)) => handle_update(ctx, entity_name, id, body),
            (HttpMethod::Delete, Some(id)) => handle_delete(ctx, entity_name, id),
            _ => (405, json_error("METHOD_NOT_ALLOWED", "Method not allowed")),
        });
    }

    None
}
