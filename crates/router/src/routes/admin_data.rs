//! Admin-only bulk data endpoints: export (full or per-entity) and
//! import. Both intentionally bypass entity policies — operator
//! escape hatches for migrations, backups, fixups. Do NOT proxy
//! user-supplied data through these.

use crate::{
    chrono_now_iso, json_error, json_error_safe, parse_json, require_admin, RouterContext,
};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    // GET /api/export — full database dump
    if url == "/api/export" && method == HttpMethod::Get {
        if !ctx.auth_ctx.is_admin {
            return Some((
                403,
                json_error("FORBIDDEN", "Admin access required for data export"),
            ));
        }
        let manifest = ctx.store.manifest();
        let mut entities_map = serde_json::Map::new();
        let mut counts_map = serde_json::Map::new();
        for ent in &manifest.entities {
            match ctx.store.list(&ent.name) {
                Ok(rows) => {
                    counts_map.insert(ent.name.clone(), serde_json::json!(rows.len()));
                    entities_map.insert(ent.name.clone(), serde_json::json!(rows));
                }
                Err(e) => {
                    return Some((
                        500,
                        json_error_safe(
                            "EXPORT_FAILED",
                            "Export operation failed",
                            &format!("Failed to export {}: {}", ent.name, e.message),
                        ),
                    ));
                }
            }
        }
        let now = chrono_now_iso();
        return Some((
            200,
            serde_json::json!({
                "exported_at": now,
                "entities": entities_map,
                "counts": counts_map,
            })
            .to_string(),
        ));
    }

    // GET /api/export/<entity> — per-entity dump
    if let Some(entity_name) = url.strip_prefix("/api/export/") {
        let entity_name = entity_name.split('?').next().unwrap_or(entity_name);
        if method == HttpMethod::Get && !entity_name.is_empty() {
            if !ctx.auth_ctx.is_admin {
                return Some((
                    403,
                    json_error("FORBIDDEN", "Admin access required for data export"),
                ));
            }
            return Some(match ctx.store.list(entity_name) {
                Ok(rows) => {
                    let now = chrono_now_iso();
                    let mut entities_map = serde_json::Map::new();
                    let mut counts_map = serde_json::Map::new();
                    counts_map.insert(entity_name.to_string(), serde_json::json!(rows.len()));
                    entities_map.insert(entity_name.to_string(), serde_json::json!(rows));
                    (
                        200,
                        serde_json::json!({
                            "exported_at": now,
                            "entities": entities_map,
                            "counts": counts_map,
                        })
                        .to_string(),
                    )
                }
                Err(e) => (400, json_error(&e.code, &e.message)),
            });
        }
    }

    // POST /api/import — load a backup bundle
    if url == "/api/import" && method == HttpMethod::Post {
        if let Some(err) = require_admin(ctx) {
            return Some(err);
        }
        if !ctx.auth_ctx.is_admin {
            return Some((
                403,
                json_error("FORBIDDEN", "Admin access required for data import"),
            ));
        }
        let data: serde_json::Value = match parse_json(body) {
            Ok(v) => v,
            Err((s, b)) => return Some((s, b)),
        };
        let entities_obj = match data.get("entities").and_then(|v| v.as_object()) {
            Some(o) => o,
            None => {
                return Some((
                    400,
                    json_error("MISSING_FIELD", "Import requires `entities` object"),
                ));
            }
        };

        let mut report: Vec<serde_json::Value> = Vec::new();
        let mut total_inserted: u64 = 0;
        let mut total_failed: u64 = 0;

        for (entity_name, rows_value) in entities_obj {
            let rows = match rows_value.as_array() {
                Some(a) => a,
                None => continue,
            };
            let mut inserted = 0u64;
            let mut failed = 0u64;
            for row in rows {
                let mut data = row.clone();
                if let Some(obj) = data.as_object_mut() {
                    obj.remove("__internal__");
                }
                match ctx.store.insert(entity_name, &data) {
                    Ok(_) => inserted += 1,
                    Err(_) => failed += 1,
                }
            }
            total_inserted += inserted;
            total_failed += failed;
            report.push(serde_json::json!({
                "entity": entity_name,
                "inserted": inserted,
                "failed": failed,
            }));
        }

        return Some((
            200,
            serde_json::json!({
                "imported": total_inserted,
                "failed": total_failed,
                "by_entity": report,
            })
            .to_string(),
        ));
    }

    None
}
