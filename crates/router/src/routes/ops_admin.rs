//! `GET /api/admin/ops/{jobs|workflows|scheduler|search_indexes}` —
//! read-only listing of the framework's operational state. Powers the
//! Studio "Operations" section so operators can debug stuck jobs,
//! see scheduled tasks, and inspect which entities have search
//! configured without dropping into psql / sqlite3.
//!
//! Admin-gated. Mirrors the auth_admin.rs pattern. No mutation surface
//! here — re-enqueue/retry/cancel each have dedicated endpoints under
//! `/api/jobs/*` and `/api/workflows/*` already (also admin-gated).

use crate::{json_error, require_admin, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    _body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    let tail = url.strip_prefix("/api/admin/ops/")?;
    if method != HttpMethod::Get {
        return Some((
            405,
            json_error("METHOD_NOT_ALLOWED", "Only GET is supported here"),
        ));
    }
    if let Some(err) = require_admin(ctx) {
        return Some(err);
    }

    let body = match tail {
        "jobs" => jobs_view(ctx),
        "workflows" => workflows_view(ctx),
        "scheduler" => scheduler_view(ctx),
        "search_indexes" => search_indexes_view(ctx),
        other => {
            return Some((
                404,
                json_error(
                    "UNKNOWN_OPS_TABLE",
                    &format!(
                        "Unknown ops table \"{other}\". Valid: jobs, workflows, scheduler, search_indexes"
                    ),
                ),
            ));
        }
    };

    Some((
        200,
        serde_json::to_string(&body).unwrap_or_else(|_| "[]".into()),
    ))
}

/// All jobs across queues + statuses. The trait already returns a
/// JSON value; we just unwrap it into the rows array Studio expects.
/// Limit is generous (1000) — the UI can paginate later if anyone hits
/// it.
fn jobs_view(ctx: &RouterContext) -> serde_json::Value {
    let raw = ctx.jobs.list_jobs(None, None, 1000);
    extract_rows(raw)
}

fn workflows_view(ctx: &RouterContext) -> serde_json::Value {
    let raw = ctx.workflows.list(None);
    extract_rows(raw)
}

/// Scheduled cron-style tasks. Returns the list as-is; each entry is
/// a `{name, expr, last_run, next_run}` shape from `Scheduler::list_tasks`.
fn scheduler_view(ctx: &RouterContext) -> serde_json::Value {
    let raw = ctx.scheduler.list_tasks();
    extract_rows(raw)
}

/// Searchable entities — sourced directly from the manifest. Each row
/// describes one entity that has a `search:` block, with the configured
/// text/facets/sortable fields. Operators can use this to confirm
/// which entities will surface in `query_filtered($search)` results.
fn search_indexes_view(ctx: &RouterContext) -> serde_json::Value {
    let manifest = ctx.store.manifest();
    let mut rows = Vec::new();
    for ent in &manifest.entities {
        let Some(cfg) = &ent.search else { continue };
        if cfg.is_empty() {
            continue;
        }
        rows.push(serde_json::json!({
            "id": ent.name.clone(),
            "entity": ent.name.clone(),
            "text_fields": cfg.text.clone(),
            "facet_fields": cfg.facets.clone(),
            "sortable_fields": cfg.sortable.clone(),
            "fts_table": format!("_fts_{}", ent.name),
        }));
    }
    serde_json::Value::Array(rows)
}

/// Trait `list_*` methods return `{rows: [...]}` or `[...]` depending
/// on the impl. Normalize so Studio always sees an array.
fn extract_rows(v: serde_json::Value) -> serde_json::Value {
    if let Some(arr) = v.as_array() {
        return serde_json::Value::Array(arr.clone());
    }
    if let Some(obj) = v.as_object() {
        if let Some(rows) = obj.get("rows") {
            return rows.clone();
        }
        if let Some(jobs) = obj.get("jobs") {
            return jobs.clone();
        }
        // Some impls return `{stats: ..., active: [...]}` — fall back
        // to the whole object as a single-element array so the operator
        // sees something rather than nothing.
        return serde_json::Value::Array(vec![v]);
    }
    serde_json::Value::Array(Vec::new())
}
