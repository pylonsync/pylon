use agentdb_core::{Diagnostic, Severity};

// ---------------------------------------------------------------------------
// Canonical schema model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub entities: Vec<Entity>,
}

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
                    message: format!(
                        "Field has an empty name in entity \"{}\"",
                        entity.name
                    ),
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
                    hint: Some(
                        "Field names must be unique within an entity".into(),
                    ),
                });
            }
        }
    }

    diagnostics
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
        };
        let diags = validate(&schema);
        assert!(diags.is_empty(), "expected no diagnostics, got: {diags:?}");
    }

    #[test]
    fn empty_schema_produces_error() {
        let schema = Schema {
            entities: vec![],
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "SCHEMA_EMPTY"));
    }

    #[test]
    fn duplicate_entity_name() {
        let schema = Schema {
            entities: vec![
                Entity { name: "Post".into(), fields: vec![], indexes: vec![] },
                Entity { name: "Post".into(), fields: vec![], indexes: vec![] },
            ],
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
        };
        let diags = validate(&schema);
        assert!(diags.iter().any(|d| d.code == "FIELD_NAME_EMPTY"));
    }
}
