//! Batched PostgreSQL graph includes with a separate child limit per key.

#![cfg(feature = "postgres-live")]

use std::collections::{HashMap, HashSet};

use pylon_http::DataError;
use pylon_kernel::{AppManifest, ManifestEntity, ManifestRelation};
use serde_json::Value;

use crate::pg_exec::PgConn;
use crate::postgres::quote_ident_pub as quote_ident;

pub(crate) fn expand_includes(
    mut rows: Vec<Value>,
    entity: &ManifestEntity,
    include: &serde_json::Map<String, Value>,
    mut fetch: impl FnMut(&ManifestRelation, &[String]) -> Result<Vec<Value>, DataError>,
) -> Vec<Value> {
    for (name, _) in include {
        let Some(relation) = entity.relations.iter().find(|r| r.name == *name) else {
            continue;
        };
        let parent_keys: Vec<Option<String>> = rows
            .iter_mut()
            .map(|row| {
                let key = row
                    .get(&relation.field)
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(object) = row.as_object_mut() {
                    object.remove(name);
                }
                key
            })
            .collect();
        let mut seen = HashSet::new();
        let keys: Vec<String> = parent_keys
            .iter()
            .flatten()
            .filter(|key| seen.insert(key.as_str()))
            .cloned()
            .collect();
        if keys.is_empty() {
            continue;
        }
        // Existing graph reads leave the include unset when its query fails.
        let Ok(related) = fetch(relation, &keys) else {
            continue;
        };
        let join_field = if relation.many { &relation.field } else { "id" };
        let mut buckets: HashMap<String, Vec<Value>> = HashMap::new();
        for child in related {
            if let Some(key) = child.get(join_field).and_then(Value::as_str) {
                buckets.entry(key.to_owned()).or_default().push(child);
            }
        }
        for (row, key) in rows.iter_mut().zip(&parent_keys) {
            let Some(key) = key else {
                continue;
            };
            if relation.many {
                row[name] = Value::Array(buckets.get(key).cloned().unwrap_or_default());
            } else if let Some(child) = buckets.get(key).and_then(|children| children.first()) {
                row[name] = child.clone();
            }
        }
    }
    rows
}

fn relation_sql(relation: &ManifestRelation) -> String {
    let field = if relation.many { &relation.field } else { "id" };
    let limit = if relation.many {
        pylon_kernel::util::effective_query_limit(None)
    } else {
        1
    };
    // LATERAL retains the old limit for EACH foreign key. An aggregate LIMIT
    // would allow one parent's children to consume another parent's capacity.
    format!(
        "SELECT related.* FROM unnest($1::text[]) WITH ORDINALITY AS keys(fk, ordinal) \
         CROSS JOIN LATERAL (SELECT child.* FROM {} AS child \
         WHERE child.{} = keys.fk ORDER BY child.\"id\" LIMIT {limit}) AS related \
         ORDER BY keys.ordinal, related.\"id\"",
        quote_ident(&relation.target),
        quote_ident(field),
    )
}

pub(crate) fn fetch_relation<C: PgConn>(
    conn: &mut C,
    manifest: &AppManifest,
    relation: &ManifestRelation,
    keys: &[String],
) -> Result<Vec<Value>, DataError> {
    let target = manifest
        .entities
        .iter()
        .find(|e| e.name == relation.target)
        .ok_or_else(|| DataError {
            code: "ENTITY_NOT_FOUND".into(),
            message: format!("Unknown entity: \"{}\"", relation.target),
        })?;
    if relation.many
        && relation.field != "id"
        && !target.fields.iter().any(|f| f.name == relation.field)
    {
        return Err(DataError {
            code: "UNKNOWN_COLUMN".into(),
            message: format!(
                "Unknown column \"{}\" on entity \"{}\"",
                relation.field, relation.target
            ),
        });
    }
    let rows = conn
        .query(&relation_sql(relation), &[&keys])
        .map_err(|e| DataError {
            code: "PG_QUERY_FAILED".into(),
            message: e.to_string(),
        })?;
    Ok(rows.iter().map(crate::postgres::row_to_json_pub).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entity() -> ManifestEntity {
        ManifestEntity {
            name: "Parent".into(),
            relations: vec![
                ManifestRelation {
                    name: "children".into(),
                    target: "Child".into(),
                    field: "group".into(),
                    many: true,
                },
                ManifestRelation {
                    name: "owner".into(),
                    target: "Owner".into(),
                    field: "owner_id".into(),
                    many: false,
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn batches_each_relation_and_preserves_duplicates_missing_keys_and_order() {
        let mut rows: Vec<_> = (0..100)
            .map(|i| json!({"id": i.to_string(), "group": "g", "owner_id": "o"}))
            .collect();
        rows.push(json!({"id": "missing", "group": "absent", "owner_id": "absent"}));
        rows.push(json!({"id": "null", "group": null}));
        let mut calls = 0;
        let result = expand_includes(
            rows,
            &entity(),
            json!({"children": {}, "owner": {}, "unknown": {}})
                .as_object()
                .unwrap(),
            |relation, keys| {
                calls += 1;
                assert_eq!(keys.len(), 2);
                Ok(if relation.many {
                    vec![
                        json!({"id": "c1", "group": "g"}),
                        json!({"id": "c2", "group": "g"}),
                    ]
                } else {
                    vec![json!({"id": "o"})]
                })
            },
        );
        assert_eq!(calls, 2);
        for row in &result[..100] {
            assert_eq!(
                row["children"],
                json!([{"id":"c1","group":"g"},{"id":"c2","group":"g"}])
            );
            assert_eq!(row["owner"], json!({"id":"o"}));
        }
        assert_eq!(result[100]["children"], json!([]));
        assert!(result[100].get("owner").is_none());
        assert!(result[101].get("children").is_none());
        assert_eq!(result[99]["id"], "99");
    }

    #[test]
    fn retained_columns_cannot_impersonate_relations() {
        let rows = vec![
            json!({"id":"p", "group":null, "owner_id":null, "children":[{"id":"secret"}], "owner":{"id":"secret"}}),
        ];
        let result = expand_includes(
            rows,
            &entity(),
            json!({"children":{}, "owner":{}}).as_object().unwrap(),
            |_, _| panic!("null keys must not query"),
        );
        assert!(result[0].get("children").is_none());
        assert!(result[0].get("owner").is_none());
    }

    #[test]
    fn failed_relation_stays_unset_and_empty_parents_need_no_queries() {
        let include = json!({"children": {}});
        let rows = vec![json!({"id":"p", "group":"g"})];
        let result = expand_includes(
            rows.clone(),
            &entity(),
            include.as_object().unwrap(),
            |_, _| {
                Err(DataError {
                    code: "test".into(),
                    message: "test".into(),
                })
            },
        );
        assert_eq!(result, rows);
        assert!(expand_includes(
            vec![],
            &entity(),
            include.as_object().unwrap(),
            |_, _| panic!("empty includes must not query")
        )
        .is_empty());
    }
}
