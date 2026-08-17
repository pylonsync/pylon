//! `POST /api/vector-search/<entity>` — exact k-NN over a
//! `vector(dims)` field. Same aggregate-safety stance as the faceted
//! search route: entities whose read policy depends on row data are
//! refused, because top-k similarity ranks over every row and would
//! leak proximity of rows the caller can't read.

use crate::{json_error, parse_json, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    let rest = url.strip_prefix("/api/vector-search/")?;
    let entity_name = rest.split('?').next().unwrap_or(rest).trim_end_matches('/');
    if entity_name.is_empty() {
        return Some((
            400,
            json_error(
                "MISSING_ENTITY",
                "vector search path is /api/vector-search/<Entity>",
            ),
        ));
    }
    if method != HttpMethod::Post {
        return Some((
            405,
            json_error(
                "METHOD_NOT_ALLOWED",
                "vector search requires POST with a JSON body",
            ),
        ));
    }
    let query_json: serde_json::Value = match parse_json(body) {
        Ok(v) => v,
        Err((status, message)) => return Some((status, message)),
    };
    // Aggregate safety — see the module docs. Probe with `None` to
    // detect a row-dependent read policy.
    let aggregate_safe = matches!(
        ctx.policy_engine
            .check_entity_read(entity_name, ctx.auth_ctx, None),
        pylon_policy::PolicyResult::Allowed
    );
    if !aggregate_safe {
        return Some((
            403,
            json_error(
                "SEARCH_REQUIRES_ROW_INDEPENDENT_POLICY",
                &format!(
                    "Entity {entity_name} has a read policy that depends on row data. \
                     Top-k vector search ranks every row and would leak proximity of \
                     rows you can't read. Make the read policy row-independent, or \
                     search from a server function via ctx.db.vectorSearch."
                ),
            ),
        ));
    }
    Some(match ctx.store.vector_search(entity_name, &query_json) {
        Ok(mut result) => {
            // Belt-and-suspenders per-hit filter + wire projection on
            // each hit's `doc` (hits are `{ id, score, doc }`).
            if let Some(hits) = result.get_mut("hits").and_then(|v| v.as_array_mut()) {
                let manifest = ctx.store.manifest();
                let auth_user = &manifest.auth.user;
                let projected: Vec<serde_json::Value> = std::mem::take(hits)
                    .into_iter()
                    .filter_map(|mut hit| {
                        let doc = hit.get_mut("doc")?.take();
                        if !matches!(
                            ctx.policy_engine.check_entity_read(
                                entity_name,
                                ctx.auth_ctx,
                                Some(&doc)
                            ),
                            pylon_policy::PolicyResult::Allowed
                        ) {
                            return None;
                        }
                        let projected_doc =
                            crate::project_row_for_wire(manifest, auth_user, entity_name, doc);
                        hit.as_object_mut()?
                            .insert("doc".to_string(), projected_doc);
                        Some(hit)
                    })
                    .collect();
                *hits = projected;
            }
            (200, result.to_string())
        }
        Err(e) => {
            let status = match e.code.as_str() {
                "ENTITY_NOT_FOUND" => 404,
                "VECTOR_FIELD_NOT_FOUND" | "INVALID_QUERY" => 400,
                "NOT_SUPPORTED" => 501,
                _ => 500,
            };
            (status, json_error(&e.code, &e.message))
        }
    })
}
