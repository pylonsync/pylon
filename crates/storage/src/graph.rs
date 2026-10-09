//! Shared graph-query option handling for all storage adapters.

/// Build the parent filter before relation expansion.
/// Both limits remain upper bounds. The adapter applies the global query cap.
/// Non-object filters retain their existing behavior when no limit is supplied.
pub fn parent_filter(options: &serde_json::Value) -> serde_json::Value {
    let mut filter = options
        .get("where")
        .cloned()
        .unwrap_or(serde_json::json!({}));
    if let Some(limit) = options.get("limit").and_then(serde_json::Value::as_u64) {
        let nested_limit = filter.get("$limit").and_then(serde_json::Value::as_u64);
        let limit = nested_limit.map_or(limit, |nested| nested.min(limit));
        if !filter.is_object() {
            filter = serde_json::json!({});
        }
        filter["$limit"] = limit.into();
    }
    filter
}

#[cfg(test)]
mod tests {
    use super::parent_filter;
    use serde_json::json;

    #[test]
    fn graph_parent_filter_preserves_both_bounds_and_other_options() {
        for (nested, top, expected) in [(2, 5, 2), (5, 2, 2), (5, 0, 0), (0, 5, 0)] {
            assert_eq!(
                parent_filter(&json!({
                    "where": {"$limit": nested, "$offset": 3, "$order": {"id": "desc"}, "owner": "a"},
                    "limit": top,
                })),
                json!({"$limit": expected, "$offset": 3, "$order": {"id": "desc"}, "owner": "a"})
            );
        }
        assert_eq!(parent_filter(&json!({})), json!({}));
        assert_eq!(
            parent_filter(&json!({"where": {"$limit": 2}})),
            json!({"$limit": 2})
        );
        assert_eq!(
            parent_filter(&json!({"where": null, "limit": 0})),
            json!({"$limit": 0})
        );
        assert_eq!(
            parent_filter(&json!({"where": {"$limit": "bad"}, "limit": 2})),
            json!({"$limit": 2})
        );
    }
}
