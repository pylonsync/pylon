use pylon_kernel::{
    AppManifest, Diagnostic, ManifestAction, ManifestPolicy, ManifestQuery, ManifestRoute,
    Severity, MANIFEST_VERSION,
};

const VALID_SCALAR_TYPES: &[&str] = &["string", "int", "float", "bool", "datetime", "richtext"];

// ---------------------------------------------------------------------------
// Canonical schema model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub entities: Vec<Entity>,
    pub queries: Vec<ManifestQuery>,
    pub actions: Vec<ManifestAction>,
    pub policies: Vec<ManifestPolicy>,
    pub routes: Vec<ManifestRoute>,
}

const VALID_AUTH_MODES: &[&str] = &["public", "user"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    pub name: String,
    pub fields: Vec<Field>,
    pub indexes: Vec<Index>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub field_type: FieldType,
    pub optional: bool,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldType {
    String,
    Int,
    Float,
    Bool,
    Datetime,
    Id(String),
    Richtext,
}

impl std::fmt::Display for FieldType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldType::String => f.write_str("string"),
            FieldType::Int => f.write_str("int"),
            FieldType::Float => f.write_str("float"),
            FieldType::Bool => f.write_str("bool"),
            FieldType::Datetime => f.write_str("datetime"),
            FieldType::Id(target) => write!(f, "id({target})"),
            FieldType::Richtext => f.write_str("richtext"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Index {
    pub name: String,
    pub fields: Vec<String>,
    pub unique: bool,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

pub fn validate(schema: &Schema) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    if schema.entities.is_empty() {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: "SCHEMA_EMPTY".into(),
            message: "Schema has no entities".into(),
            span: None,
            hint: Some("Define at least one entity".into()),
        });
    }

    let mut seen_entity_names = std::collections::HashSet::new();
    for entity in &schema.entities {
        // Empty entity name
        if entity.name.is_empty() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "ENTITY_NAME_EMPTY".into(),
                message: "Entity has an empty name".into(),
                span: None,
                hint: Some("Give the entity a non-empty name".into()),
            });
        }

        // Duplicate entity name
        if !entity.name.is_empty() && !seen_entity_names.insert(&entity.name) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "ENTITY_DUPLICATE".into(),
                message: format!("Duplicate entity name: \"{}\"", entity.name),
                span: None,
                hint: Some("Entity names must be unique within a schema".into()),
            });
        }

        let mut seen_field_names = std::collections::HashSet::new();
        for field in &entity.fields {
            // Empty field name
            if field.name.is_empty() {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "FIELD_NAME_EMPTY".into(),
                    message: format!("Field has an empty name in entity \"{}\"", entity.name),
                    span: None,
                    hint: Some("Give the field a non-empty name".into()),
                });
            }

            // Duplicate field name
            if !field.name.is_empty() && !seen_field_names.insert(&field.name) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "FIELD_DUPLICATE".into(),
                    message: format!(
                        "Duplicate field name \"{}\" in entity \"{}\"",
                        field.name, entity.name
                    ),
                    span: None,
                    hint: Some("Field names must be unique within an entity".into()),
                });
            }
        }

        // Validate indexes
        let field_names: std::collections::HashSet<&str> =
            entity.fields.iter().map(|f| f.name.as_str()).collect();
        let mut seen_index_names = std::collections::HashSet::new();

        for index in &entity.indexes {
            if index.name.is_empty() {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "INDEX_NAME_EMPTY".into(),
                    message: format!("Index has an empty name in entity \"{}\"", entity.name),
                    span: None,
                    hint: Some("Give the index a non-empty name".into()),
                });
            }

            if !index.name.is_empty() && !seen_index_names.insert(index.name.as_str()) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "INDEX_NAME_DUPLICATE".into(),
                    message: format!(
                        "Duplicate index name \"{}\" in entity \"{}\"",
                        index.name, entity.name
                    ),
                    span: None,
                    hint: Some("Index names must be unique within an entity".into()),
                });
            }

            if index.fields.is_empty() {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "INDEX_FIELDS_EMPTY".into(),
                    message: format!(
                        "Index \"{}\" in entity \"{}\" has no fields",
                        index.name, entity.name
                    ),
                    span: None,
                    hint: Some("An index must include at least one field".into()),
                });
            }

            for index_field in &index.fields {
                if !field_names.contains(index_field.as_str()) {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        code: "INDEX_FIELD_NOT_FOUND".into(),
                        message: format!(
                            "Index \"{}\" in entity \"{}\" references unknown field \"{}\"",
                            index.name, entity.name, index_field
                        ),
                        span: None,
                        hint: Some(
                            "Index fields must reference declared fields on the entity".into(),
                        ),
                    });
                }
            }
        }
    }

    // Validate id(...) references in entity fields
    for entity in &schema.entities {
        for field in &entity.fields {
            if let FieldType::Id(ref target) = field.field_type {
                if !seen_entity_names.contains(target) {
                    diagnostics.push(Diagnostic {
                        severity: Severity::Error,
                        code: "FIELD_ID_TARGET_NOT_FOUND".into(),
                        message: format!(
                            "Field \"{}\" in entity \"{}\" references unknown entity \"{}\"",
                            field.name, entity.name, target
                        ),
                        span: None,
                        hint: Some("The target entity must be declared".into()),
                    });
                }
            }
        }
    }

    // Validate id(...) references in query/action input fields
    validate_manifest_field_id_refs(
        &schema
            .queries
            .iter()
            .flat_map(|q| {
                q.input
                    .iter()
                    .map(move |f| (&q.name, &f.name, &f.field_type))
            })
            .collect::<Vec<_>>(),
        "query",
        &seen_entity_names,
        &mut diagnostics,
    );
    validate_manifest_field_id_refs(
        &schema
            .actions
            .iter()
            .flat_map(|a| {
                a.input
                    .iter()
                    .map(move |f| (&a.name, &f.name, &f.field_type))
            })
            .collect::<Vec<_>>(),
        "action",
        &seen_entity_names,
        &mut diagnostics,
    );

    // Validate queries
    let mut seen_query_names = std::collections::HashSet::new();
    for query in &schema.queries {
        if query.name.is_empty() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "QUERY_NAME_EMPTY".into(),
                message: "Query has an empty name".into(),
                span: None,
                hint: Some("Give the query a non-empty name".into()),
            });
        }
        if !query.name.is_empty() && !seen_query_names.insert(&query.name) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "QUERY_DUPLICATE".into(),
                message: format!("Duplicate query name: \"{}\"", query.name),
                span: None,
                hint: Some("Query names must be unique".into()),
            });
        }
        validate_input_field_names(&query.input, "query", &query.name, &mut diagnostics);
    }

    // Validate actions
    let mut seen_action_names = std::collections::HashSet::new();
    for action in &schema.actions {
        if action.name.is_empty() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "ACTION_NAME_EMPTY".into(),
                message: "Action has an empty name".into(),
                span: None,
                hint: Some("Give the action a non-empty name".into()),
            });
        }
        if !action.name.is_empty() && !seen_action_names.insert(&action.name) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "ACTION_DUPLICATE".into(),
                message: format!("Duplicate action name: \"{}\"", action.name),
                span: None,
                hint: Some("Action names must be unique".into()),
            });
        }
        validate_input_field_names(&action.input, "action", &action.name, &mut diagnostics);
    }

    // Validate policies
    let entity_names: std::collections::HashSet<&str> =
        schema.entities.iter().map(|e| e.name.as_str()).collect();
    let action_names: std::collections::HashSet<&str> =
        schema.actions.iter().map(|a| a.name.as_str()).collect();

    let mut seen_policy_names = std::collections::HashSet::new();
    for policy in &schema.policies {
        if policy.name.is_empty() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "POLICY_NAME_EMPTY".into(),
                message: "Policy has an empty name".into(),
                span: None,
                hint: Some("Give the policy a non-empty name".into()),
            });
        }
        if !policy.name.is_empty() && !seen_policy_names.insert(&policy.name) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "POLICY_DUPLICATE".into(),
                message: format!("Duplicate policy name: \"{}\"", policy.name),
                span: None,
                hint: Some("Policy names must be unique".into()),
            });
        }
        // A policy must have at least one allow expression, but it can be
        // any of the per-action variants (allowRead, allowInsert,
        // allowUpdate, allowDelete, allowWrite) or the universal `allow`
        // fallback. A policy with no expressions at all is always-deny and
        // almost certainly a bug.
        let has_any_allow = !policy.allow.is_empty()
            || policy.allow_read.as_deref().is_some_and(|s| !s.is_empty())
            || policy
                .allow_insert
                .as_deref()
                .is_some_and(|s| !s.is_empty())
            || policy
                .allow_update
                .as_deref()
                .is_some_and(|s| !s.is_empty())
            || policy
                .allow_delete
                .as_deref()
                .is_some_and(|s| !s.is_empty())
            || policy.allow_write.as_deref().is_some_and(|s| !s.is_empty());
        if !has_any_allow {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "POLICY_ALLOW_EMPTY".into(),
                message: format!(
                    "Policy \"{}\" has no allow expression",
                    policy.name
                ),
                span: None,
                hint: Some(
                    "Provide at least one of allow, allowRead, allowInsert, allowUpdate, allowDelete, allowWrite"
                        .into(),
                ),
            });
        }
        if policy.entity.is_none() && policy.action.is_none() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "POLICY_NO_TARGET".into(),
                message: format!(
                    "Policy \"{}\" must target at least one of entity or action",
                    policy.name
                ),
                span: None,
                hint: Some("Set entity or action on the policy".into()),
            });
        }
        if let Some(ref entity) = policy.entity {
            if !entity_names.contains(entity.as_str()) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "POLICY_ENTITY_NOT_FOUND".into(),
                    message: format!(
                        "Policy \"{}\" references unknown entity \"{}\"",
                        policy.name, entity
                    ),
                    span: None,
                    hint: Some("The entity must be declared in the entities array".into()),
                });
            }
        }
        if let Some(ref action) = policy.action {
            if !action_names.contains(action.as_str()) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "POLICY_ACTION_NOT_FOUND".into(),
                    message: format!(
                        "Policy \"{}\" references unknown action \"{}\"",
                        policy.name, action
                    ),
                    span: None,
                    hint: Some("The action must be declared in the actions array".into()),
                });
            }
        }
    }

    // Validate routes
    let query_input_names: std::collections::HashMap<&str, std::collections::HashSet<&str>> =
        schema
            .queries
            .iter()
            .map(|q| {
                let inputs: std::collections::HashSet<&str> =
                    q.input.iter().map(|f| f.name.as_str()).collect();
                (q.name.as_str(), inputs)
            })
            .collect();

    let mut seen_paths = std::collections::HashSet::new();
    for route in &schema.routes {
        if route.path.is_empty() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "ROUTE_PATH_EMPTY".into(),
                message: "Route has an empty path".into(),
                span: None,
                hint: Some("Provide a non-empty path starting with /".into()),
            });
        } else if !route.path.starts_with('/') {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "ROUTE_PATH_NO_SLASH".into(),
                message: format!("Route path \"{}\" must start with /", route.path),
                span: None,
                hint: None,
            });
        }

        if !route.path.is_empty() && !seen_paths.insert(route.path.as_str()) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "ROUTE_PATH_DUPLICATE".into(),
                message: format!("Duplicate route path: \"{}\"", route.path),
                span: None,
                hint: Some("Each route must have a unique path".into()),
            });
        }

        if let Some(ref qname) = route.query {
            if !query_input_names.contains_key(qname.as_str()) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "ROUTE_QUERY_NOT_FOUND".into(),
                    message: format!(
                        "Route \"{}\" references unknown query \"{}\"",
                        route.path, qname
                    ),
                    span: None,
                    hint: Some("The query must be declared in the queries array".into()),
                });
            } else {
                // Cross-reference path params against query input
                let params = extract_path_params(&route.path);
                if let Some(input_names) = query_input_names.get(qname.as_str()) {
                    for param in &params {
                        if !input_names.contains(param.as_str()) {
                            diagnostics.push(Diagnostic {
                                severity: Severity::Error,
                                code: "ROUTE_PARAM_NOT_IN_QUERY".into(),
                                message: format!(
                                    "Route \"{}\" has path param \"{}\" not found in query \"{}\" input",
                                    route.path, param, qname
                                ),
                                span: None,
                                hint: Some(format!(
                                    "Add \"{}\" as an input field on query \"{}\"",
                                    param, qname
                                )),
                            });
                        }
                    }
                }
            }
        }

        if let Some(ref auth) = route.auth {
            if !VALID_AUTH_MODES.contains(&auth.as_str()) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "ROUTE_AUTH_INVALID".into(),
                    message: format!(
                        "Route \"{}\" has invalid auth mode \"{}\"",
                        route.path, auth
                    ),
                    span: None,
                    hint: Some(format!("Valid auth modes: {}", VALID_AUTH_MODES.join(", "))),
                });
            }
        }
    }

    diagnostics
}

/// Extract path parameter names from a route path.
/// Parameters are segments starting with `:`.
/// Example: `/users/:userId/todos/:todoId` -> `["userId", "todoId"]`
fn extract_path_params(path: &str) -> Vec<String> {
    path.split('/')
        .filter_map(|seg| seg.strip_prefix(':'))
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string())
        .collect()
}

/// Validate input field names for duplicates and empties.
fn validate_input_field_names(
    fields: &[pylon_kernel::ManifestField],
    parent_kind: &str,
    parent_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut seen = std::collections::HashSet::new();
    for field in fields {
        if field.name.is_empty() {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "INPUT_FIELD_NAME_EMPTY".into(),
                message: format!(
                    "Input field has an empty name in {parent_kind} \"{parent_name}\""
                ),
                span: None,
                hint: Some("Give the input field a non-empty name".into()),
            });
        }
        if !field.name.is_empty() && !seen.insert(field.name.as_str()) {
            diagnostics.push(Diagnostic {
                severity: Severity::Error,
                code: "INPUT_FIELD_DUPLICATE".into(),
                message: format!(
                    "Duplicate input field name \"{}\" in {parent_kind} \"{parent_name}\"",
                    field.name
                ),
                span: None,
                hint: Some("Input field names must be unique".into()),
            });
        }
    }
}

/// Check if a field type string is a known valid type.
/// Valid types: scalars from VALID_SCALAR_TYPES, or `id(EntityName)` with non-empty target.
fn is_known_field_type(field_type: &str) -> bool {
    if VALID_SCALAR_TYPES.contains(&field_type) {
        return true;
    }
    // Accept well-formed id(X) — target validity is checked separately.
    extract_id_target(field_type).is_some()
}

/// Validate field type strings across the entire manifest.
/// Validate the manifest version.
pub fn validate_manifest_version(manifest: &AppManifest) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if manifest.manifest_version != MANIFEST_VERSION {
        diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: "MANIFEST_VERSION_UNSUPPORTED".into(),
            message: format!(
                "Manifest version {} is not supported (expected {})",
                manifest.manifest_version, MANIFEST_VERSION
            ),
            span: None,
            hint: Some(format!(
                "Regenerate the manifest with the current SDK (version {MANIFEST_VERSION})"
            )),
        });
    }
    diagnostics
}

/// This catches invalid scalar types like "strng" or "boolean" before they are
/// silently accepted.
pub fn validate_field_types(manifest: &AppManifest) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Entity fields
    for entity in &manifest.entities {
        for field in &entity.fields {
            if !is_known_field_type(&field.field_type) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "FIELD_TYPE_UNKNOWN".into(),
                    message: format!(
                        "Field \"{}\" in entity \"{}\" has unknown type \"{}\"",
                        field.name, entity.name, field.field_type
                    ),
                    span: None,
                    hint: Some(format!(
                        "Valid types: {}, id(EntityName)",
                        VALID_SCALAR_TYPES.join(", ")
                    )),
                });
            }
        }
    }

    // Query input fields
    for query in &manifest.queries {
        for field in &query.input {
            if !is_known_field_type(&field.field_type) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "INPUT_FIELD_TYPE_UNKNOWN".into(),
                    message: format!(
                        "Input field \"{}\" in query \"{}\" has unknown type \"{}\"",
                        field.name, query.name, field.field_type
                    ),
                    span: None,
                    hint: Some(format!(
                        "Valid types: {}, id(EntityName)",
                        VALID_SCALAR_TYPES.join(", ")
                    )),
                });
            }
        }
    }

    // Action input fields
    for action in &manifest.actions {
        for field in &action.input {
            if !is_known_field_type(&field.field_type) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "INPUT_FIELD_TYPE_UNKNOWN".into(),
                    message: format!(
                        "Input field \"{}\" in action \"{}\" has unknown type \"{}\"",
                        field.name, action.name, field.field_type
                    ),
                    span: None,
                    hint: Some(format!(
                        "Valid types: {}, id(EntityName)",
                        VALID_SCALAR_TYPES.join(", ")
                    )),
                });
            }
        }
    }

    diagnostics
}

/// Extract the target entity name from an `id(EntityName)` type string.
/// Returns `None` for non-id types or malformed patterns.
fn extract_id_target(field_type: &str) -> Option<&str> {
    let s = field_type.strip_prefix("id(")?;
    let s = s.strip_suffix(')')?;
    if s.is_empty() {
        return None;
    }
    Some(s)
}

/// Validate id(...) references in manifest-level fields (query/action inputs).
fn validate_manifest_field_id_refs(
    fields: &[(&String, &String, &String)], // (parent_name, field_name, field_type)
    parent_kind: &str,
    entity_names: &std::collections::HashSet<&String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (parent_name, field_name, field_type) in fields {
        if let Some(target) = extract_id_target(field_type) {
            // Check if any entity matches this target name.
            if !entity_names.iter().any(|e| e.as_str() == target) {
                diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "FIELD_ID_TARGET_NOT_FOUND".into(),
                    message: format!(
                        "Field \"{}\" in {} \"{}\" references unknown entity \"{}\"",
                        field_name, parent_kind, parent_name, target
                    ),
                    span: None,
                    hint: Some("The target entity must be declared".into()),
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_field(name: &str, ft: FieldType) -> Field {
        Field {
            name: name.into(),
            field_type: ft,
            optional: false,
            unique: false,
        }
    }

    #[test]
    fn valid_schema_produces_no_diagnostics() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![
                    make_field("title", FieldType::String),
                    make_field("body", FieldType::Richtext),
                ],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn empty_schema_produces_error() {
        let schema = Schema {
            entities: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "SCHEMA_EMPTY"));
    }

    #[test]
    fn duplicate_entity_name() {
        let schema = Schema {
            entities: vec![
                Entity {
                    name: "Post".into(),
                    fields: vec![],
                    indexes: vec![],
                },
                Entity {
                    name: "Post".into(),
                    fields: vec![],
                    indexes: vec![],
                },
            ],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "ENTITY_DUPLICATE"));
    }

    #[test]
    fn duplicate_field_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![
                    make_field("title", FieldType::String),
                    make_field("title", FieldType::String),
                ],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "FIELD_DUPLICATE"));
    }

    #[test]
    fn empty_entity_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "".into(),
                fields: vec![],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "ENTITY_NAME_EMPTY"));
    }

    #[test]
    fn empty_field_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![make_field("", FieldType::String)],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "FIELD_NAME_EMPTY"));
    }

    #[test]
    fn duplicate_query_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "X".into(),
                fields: vec![],
                indexes: vec![],
            }],
            queries: vec![
                ManifestQuery {
                    name: "getPost".into(),
                    input: vec![],
                },
                ManifestQuery {
                    name: "getPost".into(),
                    input: vec![],
                },
            ],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "QUERY_DUPLICATE"));
    }

    #[test]
    fn duplicate_action_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "X".into(),
                fields: vec![],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![
                ManifestAction {
                    name: "createPost".into(),
                    input: vec![],
                },
                ManifestAction {
                    name: "createPost".into(),
                    input: vec![],
                },
            ],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "ACTION_DUPLICATE"));
    }

    #[test]
    fn empty_query_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "X".into(),
                fields: vec![],
                indexes: vec![],
            }],
            queries: vec![ManifestQuery {
                name: "".into(),
                input: vec![],
            }],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "QUERY_NAME_EMPTY"));
    }

    #[test]
    fn empty_action_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "X".into(),
                fields: vec![],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![ManifestAction {
                name: "".into(),
                input: vec![],
            }],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "ACTION_NAME_EMPTY"));
    }

    fn make_policy(
        name: &str,
        entity: Option<&str>,
        action: Option<&str>,
        allow: &str,
    ) -> ManifestPolicy {
        ManifestPolicy {
            name: name.into(),
            entity: entity.map(|s| s.into()),
            action: action.map(|s| s.into()),
            allow: allow.into(),
        }
    }

    fn base_schema() -> Schema {
        Schema {
            entities: vec![Entity {
                name: "X".into(),
                fields: vec![],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        }
    }

    #[test]
    fn valid_policy() {
        let mut s = base_schema();
        s.policies = vec![make_policy(
            "canRead",
            Some("X"),
            None,
            "auth.userId != null",
        )];
        let diags = validate(&s);
        assert!(!diags
            .iter()
            .any(|d| d.severity == Severity::Error && d.code.starts_with("POLICY_")));
    }

    #[test]
    fn duplicate_policy_name() {
        let mut s = base_schema();
        s.policies = vec![
            make_policy("p1", Some("X"), None, "true"),
            make_policy("p1", Some("X"), None, "true"),
        ];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "POLICY_DUPLICATE"));
    }

    #[test]
    fn empty_policy_name() {
        let mut s = base_schema();
        s.policies = vec![make_policy("", Some("X"), None, "true")];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "POLICY_NAME_EMPTY"));
    }

    #[test]
    fn empty_policy_allow() {
        let mut s = base_schema();
        s.policies = vec![make_policy("p1", Some("X"), None, "")];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "POLICY_ALLOW_EMPTY"));
    }

    #[test]
    fn policy_no_target() {
        let mut s = base_schema();
        s.policies = vec![make_policy("p1", None, None, "true")];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "POLICY_NO_TARGET"));
    }

    #[test]
    fn route_with_valid_query_and_auth() {
        let mut s = base_schema();
        s.queries = vec![ManifestQuery {
            name: "allX".into(),
            input: vec![],
        }];
        s.routes = vec![ManifestRoute {
            path: "/x".into(),
            mode: "server".into(),
            query: Some("allX".into()),
            auth: Some("user".into()),
        }];
        let diags = validate(&s);
        assert!(!diags.iter().any(|d| d.code.starts_with("ROUTE_")));
    }

    #[test]
    fn route_references_unknown_query() {
        let mut s = base_schema();
        s.routes = vec![ManifestRoute {
            path: "/x".into(),
            mode: "server".into(),
            query: Some("doesNotExist".into()),
            auth: None,
        }];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "ROUTE_QUERY_NOT_FOUND"));
    }

    #[test]
    fn route_invalid_auth_mode() {
        let mut s = base_schema();
        s.routes = vec![ManifestRoute {
            path: "/x".into(),
            mode: "server".into(),
            query: None,
            auth: Some("admin".into()),
        }];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "ROUTE_AUTH_INVALID"));
    }

    #[test]
    fn route_public_auth_is_valid() {
        let mut s = base_schema();
        s.routes = vec![ManifestRoute {
            path: "/".into(),
            mode: "server".into(),
            query: None,
            auth: Some("public".into()),
        }];
        let diags = validate(&s);
        assert!(!diags.iter().any(|d| d.code.starts_with("ROUTE_")));
    }

    #[test]
    fn policy_unknown_entity() {
        let mut s = base_schema();
        s.policies = vec![ManifestPolicy {
            name: "p1".into(),
            entity: Some("NonExistent".into()),
            action: None,
            allow: "true".into(),
        }];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "POLICY_ENTITY_NOT_FOUND"));
    }

    #[test]
    fn policy_unknown_action() {
        let mut s = base_schema();
        s.policies = vec![ManifestPolicy {
            name: "p1".into(),
            entity: None,
            action: Some("nonExistentAction".into()),
            allow: "true".into(),
        }];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "POLICY_ACTION_NOT_FOUND"));
    }

    #[test]
    fn policy_valid_entity_ref() {
        let mut s = base_schema();
        s.policies = vec![ManifestPolicy {
            name: "p1".into(),
            entity: Some("X".into()),
            action: None,
            allow: "true".into(),
        }];
        let diags = validate(&s);
        assert!(!diags
            .iter()
            .any(|d| d.code.starts_with("POLICY_ENTITY") || d.code.starts_with("POLICY_ACTION")));
    }

    #[test]
    fn policy_valid_action_ref() {
        let mut s = base_schema();
        s.actions = vec![ManifestAction {
            name: "doThing".into(),
            input: vec![],
        }];
        s.policies = vec![ManifestPolicy {
            name: "p1".into(),
            entity: None,
            action: Some("doThing".into()),
            allow: "true".into(),
        }];
        let diags = validate(&s);
        assert!(!diags
            .iter()
            .any(|d| d.code.starts_with("POLICY_ENTITY") || d.code.starts_with("POLICY_ACTION")));
    }

    #[test]
    fn policy_valid_both_entity_and_action() {
        let mut s = base_schema();
        s.actions = vec![ManifestAction {
            name: "doThing".into(),
            input: vec![],
        }];
        s.policies = vec![ManifestPolicy {
            name: "p1".into(),
            entity: Some("X".into()),
            action: Some("doThing".into()),
            allow: "true".into(),
        }];
        let diags = validate(&s);
        assert!(!diags.iter().any(|d| d.code.starts_with("POLICY_")));
    }

    fn make_route(path: &str, query: Option<&str>, auth: Option<&str>) -> ManifestRoute {
        ManifestRoute {
            path: path.into(),
            mode: "server".into(),
            query: query.map(|s| s.into()),
            auth: auth.map(|s| s.into()),
        }
    }

    #[test]
    fn route_duplicate_path() {
        let mut s = base_schema();
        s.routes = vec![make_route("/x", None, None), make_route("/x", None, None)];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "ROUTE_PATH_DUPLICATE"));
    }

    #[test]
    fn route_empty_path() {
        let mut s = base_schema();
        s.routes = vec![make_route("", None, None)];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "ROUTE_PATH_EMPTY"));
    }

    #[test]
    fn route_path_no_leading_slash() {
        let mut s = base_schema();
        s.routes = vec![make_route("todos", None, None)];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "ROUTE_PATH_NO_SLASH"));
    }

    #[test]
    fn extract_params_from_path() {
        assert_eq!(extract_path_params("/"), Vec::<String>::new());
        assert_eq!(extract_path_params("/todos"), Vec::<String>::new());
        assert_eq!(extract_path_params("/todos/:todoId"), vec!["todoId"]);
        assert_eq!(
            extract_path_params("/users/:userId/todos/:todoId"),
            vec!["userId", "todoId"]
        );
    }

    #[test]
    fn route_param_missing_from_query_input() {
        let mut s = base_schema();
        s.queries = vec![ManifestQuery {
            name: "getX".into(),
            input: vec![],
        }];
        s.routes = vec![make_route("/x/:xId", Some("getX"), None)];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "ROUTE_PARAM_NOT_IN_QUERY"));
    }

    #[test]
    fn route_param_present_in_query_input() {
        let mut s = base_schema();
        s.queries = vec![ManifestQuery {
            name: "getX".into(),
            input: vec![pylon_kernel::ManifestField {
                name: "xId".into(),
                field_type: "id(X)".into(),
                optional: false,
                unique: false,
            }],
        }];
        s.routes = vec![make_route("/x/:xId", Some("getX"), None)];
        let diags = validate(&s);
        assert!(!diags.iter().any(|d| d.code == "ROUTE_PARAM_NOT_IN_QUERY"));
    }

    #[test]
    fn index_duplicate_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![make_field("title", FieldType::String)],
                indexes: vec![
                    Index {
                        name: "idx".into(),
                        fields: vec!["title".into()],
                        unique: false,
                    },
                    Index {
                        name: "idx".into(),
                        fields: vec!["title".into()],
                        unique: false,
                    },
                ],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "INDEX_NAME_DUPLICATE"));
    }

    #[test]
    fn index_unknown_field() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![make_field("title", FieldType::String)],
                indexes: vec![Index {
                    name: "idx".into(),
                    fields: vec!["nonexistent".into()],
                    unique: false,
                }],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "INDEX_FIELD_NOT_FOUND"));
    }

    #[test]
    fn index_empty_fields() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![make_field("title", FieldType::String)],
                indexes: vec![Index {
                    name: "idx".into(),
                    fields: vec![],
                    unique: false,
                }],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "INDEX_FIELDS_EMPTY"));
    }

    #[test]
    fn index_valid_multi_field() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![
                    make_field("authorId", FieldType::Id("User".into())),
                    make_field("createdAt", FieldType::Datetime),
                ],
                indexes: vec![Index {
                    name: "by_author_date".into(),
                    fields: vec!["authorId".into(), "createdAt".into()],
                    unique: false,
                }],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(!diags.iter().any(|d| d.code.starts_with("INDEX_")));
    }

    #[test]
    fn index_empty_name() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![make_field("title", FieldType::String)],
                indexes: vec![Index {
                    name: "".into(),
                    fields: vec!["title".into()],
                    unique: false,
                }],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "INDEX_NAME_EMPTY"));
    }

    #[test]
    fn unique_field_is_valid_without_index() {
        let schema = Schema {
            entities: vec![Entity {
                name: "User".into(),
                fields: vec![Field {
                    name: "email".into(),
                    field_type: FieldType::String,
                    optional: false,
                    unique: true,
                }],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(!diags.iter().any(|d| d.severity == Severity::Error));
    }

    #[test]
    fn entity_field_valid_id_ref() {
        let schema = Schema {
            entities: vec![
                Entity {
                    name: "User".into(),
                    fields: vec![],
                    indexes: vec![],
                },
                Entity {
                    name: "Post".into(),
                    fields: vec![make_field("authorId", FieldType::Id("User".into()))],
                    indexes: vec![],
                },
            ],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(!diags.iter().any(|d| d.code == "FIELD_ID_TARGET_NOT_FOUND"));
    }

    #[test]
    fn entity_field_invalid_id_ref() {
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![make_field("authorId", FieldType::Id("Missing".into()))],
                indexes: vec![],
            }],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "FIELD_ID_TARGET_NOT_FOUND"));
    }

    #[test]
    fn query_input_invalid_id_ref() {
        let mut s = base_schema();
        s.queries = vec![ManifestQuery {
            name: "getPost".into(),
            input: vec![pylon_kernel::ManifestField {
                name: "authorId".into(),
                field_type: "id(Missing)".into(),
                optional: false,
                unique: false,
            }],
        }];
        let diags = validate(&s);
        assert!(diags
            .iter()
            .any(|d| d.code == "FIELD_ID_TARGET_NOT_FOUND" && d.message.contains("query")));
    }

    #[test]
    fn action_input_invalid_id_ref() {
        let mut s = base_schema();
        s.actions = vec![ManifestAction {
            name: "doThing".into(),
            input: vec![pylon_kernel::ManifestField {
                name: "targetId".into(),
                field_type: "id(Nope)".into(),
                optional: false,
                unique: false,
            }],
        }];
        let diags = validate(&s);
        assert!(diags
            .iter()
            .any(|d| d.code == "FIELD_ID_TARGET_NOT_FOUND" && d.message.contains("action")));
    }

    #[test]
    fn malformed_id_type_is_ignored() {
        // Malformed id( without closing paren is treated as opaque, not validated.
        let schema = Schema {
            entities: vec![Entity {
                name: "Post".into(),
                fields: vec![Field {
                    name: "weirdField".into(),
                    field_type: FieldType::String, // entity fields use FieldType enum, so malformed only applies to manifest strings
                    optional: false,
                    unique: false,
                }],
                indexes: vec![],
            }],
            queries: vec![ManifestQuery {
                name: "q".into(),
                input: vec![pylon_kernel::ManifestField {
                    name: "x".into(),
                    field_type: "id(".into(), // malformed
                    optional: false,
                    unique: false,
                }],
            }],
            actions: vec![],
            policies: vec![],
            routes: vec![],
        };
        let diags = validate(&schema);
        // No FIELD_ID_TARGET_NOT_FOUND because malformed id( is ignored
        assert!(!diags.iter().any(|d| d.code == "FIELD_ID_TARGET_NOT_FOUND"));
    }

    #[test]
    fn extract_id_target_works() {
        assert_eq!(extract_id_target("id(User)"), Some("User"));
        assert_eq!(extract_id_target("id(Post)"), Some("Post"));
        assert_eq!(extract_id_target("string"), None);
        assert_eq!(extract_id_target("id("), None);
        assert_eq!(extract_id_target("id()"), None);
        assert_eq!(extract_id_target("id"), None);
    }

    #[test]
    fn field_type_display() {
        assert_eq!(format!("{}", FieldType::String), "string");
        assert_eq!(format!("{}", FieldType::Int), "int");
        assert_eq!(format!("{}", FieldType::Float), "float");
        assert_eq!(format!("{}", FieldType::Bool), "bool");
        assert_eq!(format!("{}", FieldType::Datetime), "datetime");
        assert_eq!(format!("{}", FieldType::Richtext), "richtext");
        assert_eq!(format!("{}", FieldType::Id("User".into())), "id(User)");
    }

    fn make_manifest_field(name: &str, ft: &str, optional: bool) -> pylon_kernel::ManifestField {
        pylon_kernel::ManifestField {
            name: name.into(),
            field_type: ft.into(),
            optional,
            unique: false,
        }
    }

    #[test]
    fn query_input_duplicate_field_name() {
        let mut s = base_schema();
        s.queries = vec![ManifestQuery {
            name: "q".into(),
            input: vec![
                make_manifest_field("x", "string", false),
                make_manifest_field("x", "int", false),
            ],
        }];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "INPUT_FIELD_DUPLICATE"));
    }

    #[test]
    fn action_input_empty_field_name() {
        let mut s = base_schema();
        s.actions = vec![ManifestAction {
            name: "a".into(),
            input: vec![make_manifest_field("", "string", false)],
        }];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "INPUT_FIELD_NAME_EMPTY"));
    }

    #[test]
    fn optional_id_ref_still_validated() {
        let mut s = base_schema();
        s.queries = vec![ManifestQuery {
            name: "q".into(),
            input: vec![make_manifest_field("ref", "id(Missing)", true)],
        }];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "FIELD_ID_TARGET_NOT_FOUND"));
    }

    #[test]
    fn duplicate_optional_and_required_input_field_names() {
        let mut s = base_schema();
        s.actions = vec![ManifestAction {
            name: "a".into(),
            input: vec![
                make_manifest_field("x", "string", false),
                make_manifest_field("x", "string", true),
            ],
        }];
        let diags = validate(&s);
        assert!(diags.iter().any(|d| d.code == "INPUT_FIELD_DUPLICATE"));
    }

    // -- Field type validation tests --

    fn make_manifest_entity(
        name: &str,
        fields: Vec<pylon_kernel::ManifestField>,
    ) -> pylon_kernel::ManifestEntity {
        pylon_kernel::ManifestEntity {
            name: name.into(),
            fields,
            indexes: vec![],
            relations: vec![],
        }
    }

    fn make_test_manifest(entities: Vec<pylon_kernel::ManifestEntity>) -> AppManifest {
        AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities,
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        }
    }

    #[test]
    fn valid_scalar_types_accepted() {
        let m = make_test_manifest(vec![make_manifest_entity(
            "X",
            vec![
                make_manifest_field("a", "string", false),
                make_manifest_field("b", "int", false),
                make_manifest_field("c", "float", false),
                make_manifest_field("d", "bool", false),
                make_manifest_field("e", "datetime", false),
                make_manifest_field("f", "richtext", false),
            ],
        )]);
        let diags = validate_field_types(&m);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn invalid_entity_field_type() {
        let m = make_test_manifest(vec![make_manifest_entity(
            "X",
            vec![make_manifest_field("a", "strng", false)],
        )]);
        let diags = validate_field_types(&m);
        assert!(diags.iter().any(|d| d.code == "FIELD_TYPE_UNKNOWN"));
    }

    #[test]
    fn invalid_query_input_type() {
        let mut m = make_test_manifest(vec![make_manifest_entity("X", vec![])]);
        m.queries = vec![pylon_kernel::ManifestQuery {
            name: "q".into(),
            input: vec![make_manifest_field("x", "boolean", false)],
        }];
        let diags = validate_field_types(&m);
        assert!(diags
            .iter()
            .any(|d| d.code == "INPUT_FIELD_TYPE_UNKNOWN" && d.message.contains("query")));
    }

    #[test]
    fn invalid_action_input_type() {
        let mut m = make_test_manifest(vec![make_manifest_entity("X", vec![])]);
        m.actions = vec![pylon_kernel::ManifestAction {
            name: "a".into(),
            input: vec![make_manifest_field("x", "date_time", false)],
        }];
        let diags = validate_field_types(&m);
        assert!(diags
            .iter()
            .any(|d| d.code == "INPUT_FIELD_TYPE_UNKNOWN" && d.message.contains("action")));
    }

    #[test]
    fn valid_id_type_accepted() {
        let m = make_test_manifest(vec![make_manifest_entity(
            "X",
            vec![make_manifest_field("ref", "id(X)", false)],
        )]);
        let diags = validate_field_types(&m);
        assert!(diags.is_empty());
    }

    #[test]
    fn malformed_id_type_rejected() {
        let m = make_test_manifest(vec![make_manifest_entity(
            "X",
            vec![make_manifest_field("ref", "id(", false)],
        )]);
        let diags = validate_field_types(&m);
        assert!(diags.iter().any(|d| d.code == "FIELD_TYPE_UNKNOWN"));
    }

    #[test]
    fn is_known_field_type_works() {
        assert!(is_known_field_type("string"));
        assert!(is_known_field_type("int"));
        assert!(is_known_field_type("float"));
        assert!(is_known_field_type("bool"));
        assert!(is_known_field_type("datetime"));
        assert!(is_known_field_type("richtext"));
        assert!(is_known_field_type("id(User)"));
        assert!(!is_known_field_type("strng"));
        assert!(!is_known_field_type("boolean"));
        assert!(!is_known_field_type("id("));
        assert!(!is_known_field_type("id()"));
        assert!(!is_known_field_type("text"));
    }
}
