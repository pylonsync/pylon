//! `POST /api/search/<entity>` — faceted search over the entity's
//! search index. Refuses entities whose read policy depends on row
//! data because facet aggregates would leak counts even when hits
//! were filtered.

use crate::{json_error, parse_json, RouterContext};
use pylon_http::HttpMethod;

pub(crate) fn handle(
    ctx: &RouterContext,
    method: HttpMethod,
    url: &str,
    body: &str,
    _auth_token: Option<&str>,
) -> Option<(u16, String)> {
    let rest = url.strip_prefix("/api/search/")?;
    let entity_name = rest.split('?').next().unwrap_or(rest).trim_end_matches('/');
    if entity_name.is_empty() {
        return Some((
            400,
            json_error("MISSING_ENTITY", "search path is /api/search/<Entity>"),
        ));
    }
    if method != HttpMethod::Post {
        return Some((
            405,
            json_error(
                "METHOD_NOT_ALLOWED",
                "search requires POST with a JSON body",
            ),
        ));
    }
    let query_json: serde_json::Value = match parse_json(body) {
        Ok(v) => v,
        Err((status, message)) => return Some((status, message)),
    };
    // Refuse search on entities whose read policy depends on per-row
    // fields (e.g. `auth.userId == data.ownerId`). Facet counts +
    // totals are computed across the full match-set via bitmap
    // intersection — exposing aggregates for row-scoped data leaks
    // "how many X does tenant Y have" even if individual hits were
    // filtered. Probe with `None` to detect row-independence.
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
                     Faceted search computes aggregates over every match and would \
                     leak counts for rows you can't read. Make the read policy \
                     row-independent, or disable search: in the manifest."
                ),
            ),
        ));
    }
    Some(match ctx.store.search(entity_name, &query_json) {
        Ok(mut result) => {
            // Belt-and-suspenders per-hit filter. With aggregate_safe
            // above, the policy already allows "anyone who passes the
            // auth check" — should be a no-op here, but guards against
            // future relaxations of the aggregate_safe gate.
            if let Some(hits) = result.get_mut("hits").and_then(|v| v.as_array_mut()) {
                hits.retain(|hit| {
                    matches!(
                        ctx.policy_engine
                            .check_entity_read(entity_name, ctx.auth_ctx, Some(hit)),
                        pylon_policy::PolicyResult::Allowed
                    )
                });
            }
            (200, result.to_string())
        }
        Err(e) => {
            let status = match e.code.as_str() {
                "ENTITY_NOT_FOUND" => 404,
                "SEARCH_NOT_CONFIGURED" | "INVALID_QUERY" => 400,
                _ => 500,
            };
            (status, json_error(&e.code, &e.message))
        }
    })
}
