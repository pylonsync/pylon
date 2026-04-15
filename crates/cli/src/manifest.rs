use agentdb_core::{AppManifest, Diagnostic, Severity, Span};
use agentdb_schema::{Entity, Field, FieldType, Index, Schema};

/// Run all validation passes on a manifest.
pub fn validate_all(manifest: &AppManifest) -> Vec<Diagnostic> {
    // Check version first — if unsupported, still run other checks for maximum feedback.
    let mut diagnostics = agentdb_schema::validate_manifest_version(manifest);
    let schema = manifest_to_schema(manifest);
    diagnostics.extend(agentdb_schema::validate(&schema));
    diagnostics.extend(agentdb_schema::validate_field_types(manifest));
    diagnostics
}

/// Parse a JSON string into an AppManifest, returning structured diagnostics on failure.
pub fn parse_manifest(contents: &str, path: &str) -> Result<AppManifest, Vec<Diagnostic>> {
    serde_json::from_str(contents).map_err(|e| {
        vec![Diagnostic {
            severity: Severity::Error,
            code: "MANIFEST_PARSE_ERROR".into(),
            message: format!("Invalid manifest JSON: {e}"),
            span: Some(Span {
                file: path.into(),
                line: None,
                column: None,
            }),
            hint: Some("Ensure the manifest is valid JSON matching the canonical schema".into()),
        }]
    })
}

/// Read and parse a manifest file from disk.
pub fn load_manifest(path: &str) -> Result<AppManifest, Vec<Diagnostic>> {
    let contents = std::fs::read_to_string(path).map_err(|e| {
        vec![Diagnostic {
            severity: Severity::Error,
            code: "MANIFEST_READ_FAILED".into(),
            message: format!("Could not read manifest: {path}: {e}"),
            span: None,
            hint: Some("Provide a valid manifest path or run from the project root".into()),
        }]
    })?;
    parse_manifest(&contents, path)
}

/// Convert an AppManifest into a Schema for validation.
pub fn manifest_to_schema(manifest: &AppManifest) -> Schema {
    Schema {
        entities: manifest
            .entities
            .iter()
            .map(|e| Entity {
                name: e.name.clone(),
                fields: e
                    .fields
                    .iter()
                    .map(|f| Field {
                        name: f.name.clone(),
                        field_type: parse_field_type(&f.field_type),
                        optional: f.optional,
                        unique: f.unique,
                    })
                    .collect(),
                indexes: e
                    .indexes
                    .iter()
                    .map(|i| Index {
                        name: i.name.clone(),
                        fields: i.fields.clone(),
                        unique: i.unique,
                    })
                    .collect(),
            })
            .collect(),
        queries: manifest.queries.clone(),
        actions: manifest.actions.clone(),
        policies: manifest.policies.clone(),
        routes: manifest.routes.clone(),
    }
}

fn parse_field_type(s: &str) -> FieldType {
    match s {
        "string" => FieldType::String,
        "int" => FieldType::Int,
        "float" => FieldType::Float,
        "bool" => FieldType::Bool,
        "datetime" => FieldType::Datetime,
        "richtext" => FieldType::Richtext,
        other if other.starts_with("id(") && other.ends_with(')') => {
            FieldType::Id(other[3..other.len() - 1].to_string())
        }
        _ => FieldType::String,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_manifest() {
        let json = r#"{
            "manifest_version": 1,
            "name": "test-app",
            "version": "0.1.0",
            "entities": [{
                "name": "Post",
                "fields": [
                    {"name": "title", "type": "string", "optional": false, "unique": false},
                    {"name": "slug", "type": "string", "optional": false, "unique": true}
                ],
                "indexes": []
            }],
            "routes": [
                {"path": "/", "mode": "server"}
            ]
        }"#;
        let m = parse_manifest(json, "test.json").unwrap();
        assert_eq!(m.name, "test-app");
        assert_eq!(m.entities.len(), 1);
        assert_eq!(m.entities[0].fields.len(), 2);
        assert_eq!(m.entities[0].fields[1].unique, true);
        assert_eq!(m.routes.len(), 1);
    }

    #[test]
    fn parse_manifest_with_indexes() {
        let json = r#"{
            "manifest_version": 1,
            "name": "test",
            "version": "0.1.0",
            "entities": [{
                "name": "Todo",
                "fields": [
                    {"name": "authorId", "type": "id(User)", "optional": false, "unique": false}
                ],
                "indexes": [
                    {"name": "by_author", "fields": ["authorId"], "unique": false}
                ]
            }],
            "routes": []
        }"#;
        let m = parse_manifest(json, "test.json").unwrap();
        assert_eq!(m.entities[0].indexes.len(), 1);
        assert_eq!(m.entities[0].indexes[0].name, "by_author");
        assert_eq!(m.entities[0].indexes[0].fields, vec!["authorId"]);
    }

    #[test]
    fn parse_invalid_json_returns_diagnostic() {
        let result = parse_manifest("not json", "test.json");
        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert_eq!(diags[0].code, "MANIFEST_PARSE_ERROR");
    }

    #[test]
    fn roundtrip_manifest() {
        let json = r#"{"manifest_version":1,"name":"app","version":"1.0.0","entities":[],"routes":[]}"#;
        let m = parse_manifest(json, "test.json").unwrap();
        assert_eq!(m.manifest_version, 1);
        assert!(m.queries.is_empty());
        assert!(m.actions.is_empty());
        let serialized = serde_json::to_string(&m).unwrap();
        let m2: AppManifest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn parse_manifest_with_queries_and_actions() {
        let json = r#"{
            "manifest_version": 1,
            "name": "test",
            "version": "0.1.0",
            "entities": [{"name": "Post", "fields": [], "indexes": []}],
            "routes": [],
            "queries": [
                {"name": "getPost", "input": [{"name": "postId", "type": "id(Post)", "optional": false, "unique": false}]},
                {"name": "listPosts"}
            ],
            "actions": [
                {"name": "createPost", "input": [{"name": "title", "type": "string", "optional": false, "unique": false}]}
            ]
        }"#;
        let m = parse_manifest(json, "test.json").unwrap();
        assert_eq!(m.queries.len(), 2);
        assert_eq!(m.queries[0].name, "getPost");
        assert_eq!(m.queries[0].input.len(), 1);
        assert_eq!(m.queries[1].name, "listPosts");
        assert!(m.queries[1].input.is_empty());
        assert_eq!(m.actions.len(), 1);
        assert_eq!(m.actions[0].name, "createPost");
        assert_eq!(m.actions[0].input.len(), 1);
    }

    #[test]
    fn parse_manifest_with_policies() {
        let json = r#"{
            "manifest_version": 1,
            "name": "test",
            "version": "0.1.0",
            "entities": [{"name": "Post", "fields": [], "indexes": []}],
            "routes": [],
            "policies": [
                {"name": "ownerCanEdit", "entity": "Post", "allow": "auth.userId == data.ownerId"},
                {"name": "authCreate", "action": "createPost", "allow": "auth.userId != null"}
            ]
        }"#;
        let m = parse_manifest(json, "test.json").unwrap();
        assert_eq!(m.policies.len(), 2);
        assert_eq!(m.policies[0].name, "ownerCanEdit");
        assert_eq!(m.policies[0].entity.as_deref(), Some("Post"));
        assert!(m.policies[0].action.is_none());
        assert_eq!(m.policies[0].allow, "auth.userId == data.ownerId");
        assert_eq!(m.policies[1].name, "authCreate");
        assert!(m.policies[1].entity.is_none());
        assert_eq!(m.policies[1].action.as_deref(), Some("createPost"));
    }

    #[test]
    fn parse_manifest_with_route_query_and_auth() {
        let json = r#"{
            "manifest_version": 1,
            "name": "test",
            "version": "0.1.0",
            "entities": [{"name": "Post", "fields": [], "indexes": []}],
            "routes": [
                {"path": "/", "mode": "server"},
                {"path": "/todos", "mode": "live", "query": "allTodos", "auth": "user"}
            ]
        }"#;
        let m = parse_manifest(json, "test.json").unwrap();
        assert_eq!(m.routes.len(), 2);
        assert!(m.routes[0].query.is_none());
        assert!(m.routes[0].auth.is_none());
        assert_eq!(m.routes[1].query.as_deref(), Some("allTodos"));
        assert_eq!(m.routes[1].auth.as_deref(), Some("user"));
    }

    #[test]
    fn parse_manifest_optional_input_field() {
        let json = r#"{
            "manifest_version": 1,
            "name": "test",
            "version": "0.1.0",
            "entities": [{"name": "X", "fields": [], "indexes": []}],
            "routes": [],
            "queries": [
                {"name": "q", "input": [
                    {"name": "a", "type": "string", "optional": true, "unique": false},
                    {"name": "b", "type": "string", "optional": false, "unique": false}
                ]}
            ],
            "actions": [
                {"name": "a", "input": [
                    {"name": "x", "type": "id(X)", "optional": true, "unique": false}
                ]}
            ]
        }"#;
        let m = parse_manifest(json, "test.json").unwrap();
        assert_eq!(m.queries[0].input[0].optional, true);
        assert_eq!(m.queries[0].input[1].optional, false);
        assert_eq!(m.actions[0].input[0].optional, true);

        // Roundtrip preserves optional
        let serialized = serde_json::to_string(&m).unwrap();
        let m2: AppManifest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn missing_manifest_version_fails_parse() {
        let json = r#"{"name":"app","version":"1.0.0","entities":[],"routes":[]}"#;
        let result = parse_manifest(json, "test.json");
        assert!(result.is_err());
    }

    #[test]
    fn unsupported_manifest_version() {
        let json = r#"{"manifest_version":999,"name":"app","version":"1.0.0","entities":[],"routes":[]}"#;
        let m = parse_manifest(json, "test.json").unwrap();
        let diags = agentdb_schema::validate_manifest_version(&m);
        assert!(diags.iter().any(|d| d.code == "MANIFEST_VERSION_UNSUPPORTED"));
    }

    #[test]
    fn field_type_rename() {
        let json = r#"{"name":"a","type":"string","optional":false,"unique":false}"#;
        let f: agentdb_core::ManifestField = serde_json::from_str(json).unwrap();
        assert_eq!(f.field_type, "string");
        let back = serde_json::to_string(&f).unwrap();
        assert!(back.contains("\"type\":\"string\""));
    }
}
