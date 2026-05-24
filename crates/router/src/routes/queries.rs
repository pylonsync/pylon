//! Read-shaped query routes: transact (admin), filtered query, lookup,
//! aggregate, graph query. All non-admin paths apply read-policy
//! gates (entity-level + per-row).

use crate::{
    broadcast_change_with_crdt, json_error, json_error_safe, parse_json, require_admin,
    RouterContext,
};
use pylon_http::HttpMethod;
use pylon_sync::{ChangeKind, ChangeRecord};

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    // POST /api/transact (admin-only; intentionally bypasses entity policies)
    if url == "/api/transact" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        let ops: Vec<serde_json::Value> = match serde_json::from_str(body) {
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
        // Pre-snapshot targets BEFORE the transact runs:
        //   - Delete: attach the row to the broadcast so row-scoped
        //     read policies on subscribers can authorize delivery.
        //   - Update: attach the row as `prev_data` so visibility-
        //     flip subscribers (whose policy passed pre-update but
        //     denies post) get a synthesized Delete tombstone.
        let mut delete_snapshots: std::collections::HashMap<(String, String), serde_json::Value> =
            std::collections::HashMap::new();
        let mut update_pre_snapshots: std::collections::HashMap<
            (String, String),
            serde_json::Value,
        > = std::collections::HashMap::new();
        for op in &ops {
            let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
            let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");
            let id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
            if entity.is_empty() || id.is_empty() {
                continue;
            }
            match op_type {
                "delete" => {
                    if let Ok(Some(row)) = ctx.store.get_by_id(entity, id) {
                        delete_snapshots.insert((entity.to_string(), id.to_string()), row);
                    }
                }
                "update" => {
                    if let Ok(Some(row)) = ctx.store.get_by_id(entity, id) {
                        update_pre_snapshots.insert((entity.to_string(), id.to_string()), row);
                    }
                }
                _ => {}
            }
        }
        return Some(match ctx.store.transact(&ops) {
            Ok((committed, results)) => {
                // Broadcast per-op change events after the atomic
                // commit lands. Without this, /api/transact writes
                // were invisible to WS / SSE subscribers — Runtime::
                // transact (both SQLite and PG paths) writes through
                // the storage layer without touching change_log /
                // notifier. The atomic-batch guarantee from the
                // inner transact() is preserved (we only broadcast
                // after the commit succeeded); on a rollback
                // `committed` is false and we skip the per-op fanout.
                //
                // Re-read each op's row so the broadcast carries the
                // full materialized row, matching every other write
                // path's wire shape. `Ok(None)` on re-read = the row
                // was concurrently deleted; skip that op's broadcast.
                if committed {
                    // Post-commit fanout. The inner `store.transact()`
                    // is atomic; we couldn't run each op through the
                    // mutation pipeline because that would lose the
                    // all-or-nothing guarantee. Instead, after COMMIT
                    // succeeds we route each op through the same
                    // typed `ChangeRecord` + `broadcast_change_with_crdt`
                    // path the pipeline uses. Plugin `after_*` hooks
                    // run here too so transact writes observe the
                    // same plugin chain as entity-API + sync/push.
                    //
                    // Invariant: insert/update without a successful
                    // reread → skip broadcast (the row may have been
                    // concurrently deleted, or the read side errored).
                    // Don't fall back to caller payload — that would
                    // trip the per-tenant policy filter on every
                    // subscriber and ghost-apply unprojected fields.
                    for (op, result) in ops.iter().zip(results.iter()) {
                        let op_type = op.get("op").and_then(|v| v.as_str()).unwrap_or("");
                        let entity = op.get("entity").and_then(|v| v.as_str()).unwrap_or("");
                        if entity.is_empty() {
                            continue;
                        }
                        if result.get("error").is_some() {
                            continue;
                        }
                        let id = result.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        if id.is_empty() && op_type != "delete" {
                            continue;
                        }
                        match op_type {
                            "insert" | "update" => {
                                let row = match ctx.store.get_by_id(entity, id) {
                                    Ok(Some(full)) => full,
                                    Ok(None) => {
                                        tracing::warn!(
                                            "[/api/transact] post-{op_type} re-read returned None for {entity}/{id} — concurrent delete; skipping broadcast"
                                        );
                                        continue;
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "[/api/transact] post-{op_type} re-read failed for {entity}/{id} ({}): skipping broadcast",
                                            e.message
                                        );
                                        continue;
                                    }
                                };
                                let record = if op_type == "insert" {
                                    ChangeRecord::Insert { row: row.clone() }
                                } else {
                                    ChangeRecord::Update {
                                        row: row.clone(),
                                        prev: update_pre_snapshots
                                            .remove(&(entity.to_string(), id.to_string())),
                                    }
                                };
                                let event = ctx.change_log.record(entity, id, record);
                                broadcast_change_with_crdt(
                                    ctx.notifier,
                                    ctx.store,
                                    event.seq,
                                    entity,
                                    id,
                                    event.kind.clone(),
                                    event.data.as_ref(),
                                    event.prev_data.as_ref(),
                                );
                                // after_* hook with the post-write row,
                                // matching the entity-API + push path.
                                if matches!(event.kind, ChangeKind::Insert) {
                                    ctx.plugin_hooks
                                        .after_insert(entity, id, &row, ctx.auth_ctx);
                                } else {
                                    ctx.plugin_hooks
                                        .after_update(entity, id, &row, ctx.auth_ctx);
                                }
                            }
                            "delete" => {
                                let delete_id = op.get("id").and_then(|v| v.as_str()).unwrap_or("");
                                if delete_id.is_empty() {
                                    continue;
                                }
                                let snapshot = delete_snapshots
                                    .remove(&(entity.to_string(), delete_id.to_string()));
                                let event = ctx.change_log.record(
                                    entity,
                                    delete_id,
                                    ChangeRecord::Delete {
                                        snapshot: snapshot.clone(),
                                    },
                                );
                                broadcast_change_with_crdt(
                                    ctx.notifier,
                                    ctx.store,
                                    event.seq,
                                    entity,
                                    delete_id,
                                    ChangeKind::Delete,
                                    event.data.as_ref(),
                                    None,
                                );
                                ctx.plugin_hooks
                                    .after_delete(entity, delete_id, ctx.auth_ctx);
                            }
                            _ => {}
                        }
                    }
                }
                (
                    if committed { 200 } else { 400 },
                    serde_json::json!({
                        "committed": committed,
                        "results": results,
                    })
                    .to_string(),
                )
            }
            Err(e) => (500, json_error(&e.code, &e.message)),
        });
    }

    // POST /api/query/:entity (filtered)
    if url.starts_with("/api/query/") && method == HttpMethod::Post {
        let entity = url
            .strip_prefix("/api/query/")
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("");
        if !entity.is_empty() && entity != "filtered" {
            if let pylon_policy::PolicyResult::Denied {
                policy_name,
                reason,
            } = ctx
                .policy_engine
                .check_entity_read(entity, ctx.auth_ctx, None)
            {
                tracing::warn!("[policy] query {entity} denied by \"{policy_name}\": {reason}");
                return Some((
                    403,
                    crate::json_error_with_hint(
                        "POLICY_DENIED",
                        "Access denied by policy",
                        "Check your auth token or the policy rules in your schema",
                    ),
                ));
            }
            let filter: serde_json::Value = match serde_json::from_str(body) {
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
            return Some(match ctx.store.query_filtered(entity, &filter) {
                Ok(rows) => {
                    let allowed: Vec<serde_json::Value> = rows
                        .into_iter()
                        .filter(|row| {
                            matches!(
                                ctx.policy_engine.check_entity_read(
                                    entity,
                                    ctx.auth_ctx,
                                    Some(row),
                                ),
                                pylon_policy::PolicyResult::Allowed
                            )
                        })
                        .collect();
                    (
                        200,
                        serde_json::to_string(&allowed).unwrap_or_else(|_| "[]".into()),
                    )
                }
                Err(e) => (400, json_error(&e.code, &e.message)),
            });
        }
    }

    // GET /api/lookup/:entity/:field/:value
    if let Some(path) = url.strip_prefix("/api/lookup/") {
        let path = path.split('?').next().unwrap_or(path);
        let parts: Vec<&str> = path.splitn(3, '/').collect();
        if parts.len() == 3 && method == HttpMethod::Get {
            // Fetch the row FIRST, then run the policy against the
            // actual row data. Without this, per-row policies like
            // `auth.userId == data.createdBy` see `data` as null and
            // ALWAYS deny — every lookup 403s for non-admin callers.
            // Same pattern the GET /api/entities/:id/:field path uses.
            let row = match ctx.store.lookup(parts[0], parts[1], parts[2]) {
                Ok(r) => r,
                Err(e) => return Some((400, json_error(&e.code, &e.message))),
            };
            // Return 404 BEFORE the policy check when the row is
            // missing — the existence of a row at this slug isn't
            // policy-relevant, and the alternative (404 vs 403) leaks
            // less information about other tenants' rows.
            let row = match row {
                Some(r) => r,
                None => {
                    return Some((
                        404,
                        json_error(
                            "NOT_FOUND",
                            &format!("{}.{} = {} not found", parts[0], parts[1], parts[2]),
                        ),
                    ));
                }
            };
            let check = ctx
                .policy_engine
                .check_entity_read(parts[0], ctx.auth_ctx, Some(&row));
            if let pylon_policy::PolicyResult::Denied {
                policy_name,
                reason,
            } = check
            {
                tracing::warn!(
                    "[policy] lookup on {} denied by \"{policy_name}\": {reason}",
                    parts[0]
                );
                return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
            }
            return Some((
                200,
                serde_json::to_string(&row).unwrap_or_else(|_| "{}".into()),
            ));
        }
    }

    // POST /api/aggregate/:entity
    if let Some(rest) = url.strip_prefix("/api/aggregate/") {
        let entity = rest.split('?').next().unwrap_or(rest);
        if method == HttpMethod::Post && !entity.is_empty() {
            let check = ctx
                .policy_engine
                .check_entity_read(entity, ctx.auth_ctx, None);
            if let pylon_policy::PolicyResult::Denied {
                policy_name,
                reason,
            } = check
            {
                tracing::warn!(
                    "[policy] aggregate on {entity} denied by \"{policy_name}\": {reason}"
                );
                return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
            }
            let mut spec = match parse_json(body) {
                Ok(v) => v,
                Err((s, b)) => return Some((s, b)),
            };
            // Tenant clamp — if the entity has an `orgId` column and the
            // caller has an active tenant, force WHERE orgId = tenantId.
            // Server overwrites any client-supplied value, so a payload
            // can't sum cross-tenant rows.
            if let Some(tenant_id) = ctx.auth_ctx.tenant_id.as_deref() {
                let manifest = ctx.store.manifest();
                let has_org_id = manifest
                    .entities
                    .iter()
                    .find(|e| e.name == entity)
                    .map(|e| e.fields.iter().any(|f| f.name == "orgId"))
                    .unwrap_or(false);
                if has_org_id {
                    if let Some(obj) = spec.as_object_mut() {
                        let entry = obj
                            .entry("where".to_string())
                            .or_insert_with(|| serde_json::json!({}));
                        if let Some(where_obj) = entry.as_object_mut() {
                            where_obj.insert(
                                "orgId".to_string(),
                                serde_json::Value::String(tenant_id.to_string()),
                            );
                        }
                    }
                }
            }
            return Some(match ctx.store.aggregate(entity, &spec) {
                Ok(result) => (
                    200,
                    serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()),
                ),
                Err(e) => (400, json_error(&e.code, &e.message)),
            });
        }
    }

    // POST /api/query (graph)
    if url == "/api/query" && method == HttpMethod::Post {
        let query: serde_json::Value = match serde_json::from_str(body) {
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
        // Gate every entity named in the graph against the read policy.
        if let Some(obj) = query.as_object() {
            for entity_name in obj.keys() {
                let check = ctx
                    .policy_engine
                    .check_entity_read(entity_name, ctx.auth_ctx, None);
                if let pylon_policy::PolicyResult::Denied {
                    policy_name,
                    reason,
                } = check
                {
                    tracing::warn!(
                        "[policy] graph query on {entity_name} denied by \"{policy_name}\": {reason}"
                    );
                    return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
                }
            }
        }
        return Some(match ctx.store.query_graph(&query) {
            Ok(result) => (
                200,
                serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()),
            ),
            Err(e) => (400, json_error(&e.code, &e.message)),
        });
    }

    None
}
