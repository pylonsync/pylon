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
                    let manifest = ctx.store.manifest();
                    let auth_user = &manifest.auth.user;
                    let allowed: Vec<serde_json::Value> = rows
                        .into_iter()
                        .filter(|row| {
                            // Policy check runs against the RAW row
                            // so predicates referencing `serverOnly`
                            // fields evaluate against actual values.
                            matches!(
                                ctx.policy_engine.check_entity_read(
                                    entity,
                                    ctx.auth_ctx,
                                    Some(row),
                                ),
                                pylon_policy::PolicyResult::Allowed
                            )
                        })
                        // Project for the wire AFTER the policy
                        // check — strips User-entity secrets and any
                        // entity's `serverOnly` fields.
                        .map(|row| crate::project_row_for_wire(manifest, auth_user, entity, row))
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
            let manifest = ctx.store.manifest();
            let auth_user = &manifest.auth.user;
            let projected = crate::project_row_for_wire(manifest, auth_user, parts[0], row);
            return Some((
                200,
                serde_json::to_string(&projected).unwrap_or_else(|_| "{}".into()),
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
        let manifest = ctx.store.manifest();

        // Gate EVERY entity the graph can read: the named top-level
        // entities AND, recursively, the target entity of every `include`
        // relation. `query_graph` traverses relations to arbitrary target
        // entities with no `AuthContext` of its own, so the per-row read
        // policy can't run inside it — we authorize the whole shape here,
        // up front. An `include` target whose read policy is row-dependent
        // (or denies) for this caller can't be proven row-independently
        // readable with `data: None`, so the request is denied — same
        // posture as `/api/aggregate` + `/api/search` — rather than
        // leaking the target's rows (the include-bypass class).
        if let Some(obj) = query.as_object() {
            let mut touched: Vec<String> = Vec::new();
            for (entity_name, opts) in obj {
                collect_graph_entities(entity_name, opts, manifest, &mut touched);
            }
            for entity_name in &touched {
                if let pylon_policy::PolicyResult::Denied {
                    policy_name,
                    reason,
                } = ctx
                    .policy_engine
                    .check_entity_read(entity_name, ctx.auth_ctx, None)
                {
                    tracing::warn!(
                        "[policy] graph query on {entity_name} denied by \"{policy_name}\": {reason}"
                    );
                    return Some((403, json_error("POLICY_DENIED", "Access denied by policy")));
                }
            }
        }

        let mut result = match ctx.store.query_graph(&query) {
            Ok(r) => r,
            Err(e) => return Some((400, json_error(&e.code, &e.message))),
        };
        // Project every row for the wire BEFORE serialization — top-level
        // rows AND `include`d relation rows. `query_graph` returns raw
        // stored rows; without this, the User entity's `passwordHash` and
        // any `serverOnly` column leak through the graph surface even
        // though every other read path (`/api/query/:entity`, lookup,
        // entities) projects.
        project_graph_result(&mut result, &query, manifest, &manifest.auth.user);
        return Some((
            200,
            serde_json::to_string(&result).unwrap_or_else(|_| "{}".into()),
        ));
    }

    None
}

/// Collect every entity a graph query can read: the named entity plus,
/// recursively, the target entity of each `include` relation. The route
/// authorizes this whole set before dispatch because `query_graph`
/// expands relations with no policy context of its own. Dedups so a
/// shared target isn't checked twice.
fn collect_graph_entities(
    entity_name: &str,
    opts: &serde_json::Value,
    manifest: &pylon_kernel::AppManifest,
    out: &mut Vec<String>,
) {
    if !out.iter().any(|e| e == entity_name) {
        out.push(entity_name.to_string());
    }
    let Some(include) = opts.get("include").and_then(|v| v.as_object()) else {
        return;
    };
    let Some(ent) = manifest.entities.iter().find(|e| e.name == entity_name) else {
        return;
    };
    for (rel_name, sub_opts) in include {
        if let Some(rel) = ent.relations.iter().find(|r| r.name == *rel_name) {
            collect_graph_entities(&rel.target, sub_opts, manifest, out);
        }
    }
}

/// Project every row in a graph-query result for the wire: strip
/// `serverOnly` columns and User-entity secrets from the top-level rows
/// AND from every `include`d relation's rows (each keyed by relation
/// name and projected against the relation's *target* entity).
fn project_graph_result(
    result: &mut serde_json::Value,
    query: &serde_json::Value,
    manifest: &pylon_kernel::AppManifest,
    auth_user: &pylon_kernel::ManifestAuthUserConfig,
) {
    let Some(result_obj) = result.as_object_mut() else {
        return;
    };
    for (entity_name, rows_val) in result_obj.iter_mut() {
        let include_map = query
            .get(entity_name)
            .and_then(|q| q.get("include"))
            .and_then(|v| v.as_object());
        let ent = manifest.entities.iter().find(|e| e.name == *entity_name);
        let Some(rows) = rows_val.as_array_mut() else {
            continue;
        };
        for row in rows.iter_mut() {
            // Project the nested relation rows FIRST (against their target
            // entity), then the parent row. The nested values are plain
            // extra object keys, so a non-User parent projection preserves
            // them; doing nested-first guarantees the target entity's
            // `serverOnly`/secret fields are stripped regardless of order.
            if let (Some(include_map), Some(ent)) = (include_map, ent) {
                for rel_name in include_map.keys() {
                    if let Some(rel) = ent.relations.iter().find(|r| r.name == *rel_name) {
                        if let Some(slot) = row.get_mut(rel_name) {
                            project_related_in_place(slot, manifest, auth_user, &rel.target);
                        }
                    }
                }
            }
            let taken = std::mem::replace(row, serde_json::Value::Null);
            *row = crate::project_row_for_wire(manifest, auth_user, entity_name, taken);
        }
    }
}

/// Project an `include`d relation slot in place — either a single object
/// (to-one) or an array of objects (to-many) — against `target` entity.
fn project_related_in_place(
    slot: &mut serde_json::Value,
    manifest: &pylon_kernel::AppManifest,
    auth_user: &pylon_kernel::ManifestAuthUserConfig,
    target: &str,
) {
    match slot {
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                let taken = std::mem::replace(item, serde_json::Value::Null);
                *item = crate::project_row_for_wire(manifest, auth_user, target, taken);
            }
        }
        serde_json::Value::Object(_) => {
            let taken = std::mem::replace(slot, serde_json::Value::Null);
            *slot = crate::project_row_for_wire(manifest, auth_user, target, taken);
        }
        _ => {}
    }
}
