pub mod files;
#[cfg(feature = "postgres-live")]
pub mod pg_datastore;
pub mod pool;
pub mod postgres;
pub mod search;
pub mod search_maintenance;
pub mod search_query;
pub mod sqlite;

use std::fmt;

use pylon_kernel::AppManifest;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError {
    pub code: String,
    pub message: String,
}

impl StorageError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for StorageError {}

// ---------------------------------------------------------------------------
// Schema operations — what a plan is made of
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum SchemaOperation {
    CreateEntity {
        name: String,
        fields: Vec<FieldSpec>,
    },
    AddField {
        entity: String,
        field: FieldSpec,
    },
    RemoveField {
        entity: String,
        field_name: String,
    },
    RemoveEntity {
        name: String,
    },
    AddIndex {
        entity: String,
        name: String,
        fields: Vec<String>,
        unique: bool,
    },
    RemoveIndex {
        entity: String,
        name: String,
    },
    Noop,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSpec {
    pub name: String,
    pub field_type: String,
    pub optional: bool,
    pub unique: bool,
}

// ---------------------------------------------------------------------------
// Schema plan — the output of planning
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaPlan {
    pub operations: Vec<SchemaOperation>,
}

impl SchemaPlan {
    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
            || self
                .operations
                .iter()
                .all(|op| matches!(op, SchemaOperation::Noop))
    }
}

// ---------------------------------------------------------------------------
// Schema snapshot — shared introspection types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchemaSnapshot {
    pub tables: Vec<TableSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableSnapshot {
    pub name: String,
    pub columns: Vec<ColumnSnapshot>,
    pub indexes: Vec<IndexSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnSnapshot {
    pub name: String,
    pub column_type: String,
    pub notnull: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexSnapshot {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// Plan additive schema changes from a snapshot to a target manifest.
/// Shared by both SQLite and Postgres adapters.
/// Only produces CreateEntity, AddField, AddIndex, and Noop.
pub fn plan_from_snapshot(snapshot: &SchemaSnapshot, target: &AppManifest) -> SchemaPlan {
    use std::collections::{HashMap, HashSet};

    let existing_tables: HashMap<&str, &TableSnapshot> = snapshot
        .tables
        .iter()
        .map(|t| (t.name.as_str(), t))
        .collect();

    let mut operations = Vec::new();

    for entity in &target.entities {
        match existing_tables.get(entity.name.as_str()) {
            None => {
                let fields: Vec<FieldSpec> = entity
                    .fields
                    .iter()
                    .map(|f| FieldSpec {
                        name: f.name.clone(),
                        field_type: f.field_type.clone(),
                        optional: f.optional,
                        unique: f.unique,
                    })
                    .collect();
                operations.push(SchemaOperation::CreateEntity {
                    name: entity.name.clone(),
                    fields,
                });
                for index in &entity.indexes {
                    operations.push(SchemaOperation::AddIndex {
                        entity: entity.name.clone(),
                        name: index.name.clone(),
                        fields: index.fields.clone(),
                        unique: index.unique,
                    });
                }
            }
            Some(table) => {
                let existing_cols: HashSet<&str> =
                    table.columns.iter().map(|c| c.name.as_str()).collect();
                for field in &entity.fields {
                    if !existing_cols.contains(field.name.as_str()) {
                        operations.push(SchemaOperation::AddField {
                            entity: entity.name.clone(),
                            field: FieldSpec {
                                name: field.name.clone(),
                                field_type: field.field_type.clone(),
                                optional: field.optional,
                                unique: field.unique,
                            },
                        });
                    }
                }
                // Index names in DB are prefixed: {entity}_{index_name}.
                let existing_indexes: HashSet<&str> =
                    table.indexes.iter().map(|i| i.name.as_str()).collect();
                for index in &entity.indexes {
                    let full_name = format!("{}_{}", entity.name, index.name);
                    if !existing_indexes.contains(full_name.as_str()) {
                        operations.push(SchemaOperation::AddIndex {
                            entity: entity.name.clone(),
                            name: index.name.clone(),
                            fields: index.fields.clone(),
                            unique: index.unique,
                        });
                    }
                }
            }
        }
    }

    if operations.is_empty() {
        operations.push(SchemaOperation::Noop);
    }

    SchemaPlan { operations }
}

// ---------------------------------------------------------------------------
// Plan analysis — safety classification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlanAnalysis {
    pub destructive: bool,
    pub has_unsupported: bool,
    pub warnings: Vec<PlanWarning>,
}

/// Analyze a schema plan for destructive or unsupported operations.
pub fn analyze_plan(plan: &SchemaPlan) -> PlanAnalysis {
    let mut destructive = false;
    let mut has_unsupported = false;
    let mut warnings = Vec::new();

    for op in &plan.operations {
        match op {
            SchemaOperation::RemoveEntity { name } => {
                destructive = true;
                has_unsupported = true;
                warnings.push(PlanWarning {
                    code: "DESTRUCTIVE_REMOVE_ENTITY".into(),
                    message: format!(
                        "Removing entity \"{}\" will drop the table and all its data",
                        name
                    ),
                });
            }
            SchemaOperation::RemoveField { entity, field_name } => {
                destructive = true;
                has_unsupported = true;
                warnings.push(PlanWarning {
                    code: "DESTRUCTIVE_REMOVE_FIELD".into(),
                    message: format!(
                        "Removing field \"{}.{}\" will drop the column and its data",
                        entity, field_name
                    ),
                });
            }
            SchemaOperation::RemoveIndex { entity, name } => {
                has_unsupported = true;
                warnings.push(PlanWarning {
                    code: "UNSUPPORTED_REMOVE_INDEX".into(),
                    message: format!(
                        "Removing index \"{}.{}\" is not supported by the SQLite adapter",
                        entity, name
                    ),
                });
            }
            _ => {}
        }
    }

    PlanAnalysis {
        destructive,
        has_unsupported,
        warnings,
    }
}

// ---------------------------------------------------------------------------
// Storage adapter trait
// ---------------------------------------------------------------------------

pub trait StorageAdapter {
    /// Produce a plan that would bring storage in line with the target manifest.
    fn plan_schema(&self, target: &AppManifest) -> Result<SchemaPlan, StorageError>;

    /// Apply a schema plan. Not implemented by dry-run adapters.
    fn apply_schema(&self, _plan: &SchemaPlan) -> Result<(), StorageError> {
        Err(StorageError {
            code: "APPLY_NOT_IMPLEMENTED".into(),
            message: "This adapter does not support applying schemas".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Dry-run adapter — plans against an empty baseline
// ---------------------------------------------------------------------------

/// A storage adapter that assumes no existing schema.
/// It produces a plan to create everything from scratch.
pub struct DryRunAdapter;

impl StorageAdapter for DryRunAdapter {
    fn plan_schema(&self, target: &AppManifest) -> Result<SchemaPlan, StorageError> {
        let mut operations = Vec::new();

        for entity in &target.entities {
            let fields: Vec<FieldSpec> = entity
                .fields
                .iter()
                .map(|f| FieldSpec {
                    name: f.name.clone(),
                    field_type: f.field_type.clone(),
                    optional: f.optional,
                    unique: f.unique,
                })
                .collect();

            operations.push(SchemaOperation::CreateEntity {
                name: entity.name.clone(),
                fields,
            });

            for index in &entity.indexes {
                operations.push(SchemaOperation::AddIndex {
                    entity: entity.name.clone(),
                    name: index.name.clone(),
                    fields: index.fields.clone(),
                    unique: index.unique,
                });
            }
        }

        if operations.is_empty() {
            operations.push(SchemaOperation::Noop);
        }

        Ok(SchemaPlan { operations })
    }
}

// ---------------------------------------------------------------------------
// Diff-based adapter — plans from one manifest to another
// ---------------------------------------------------------------------------

/// A storage adapter that plans the transition from an old manifest to a new one.
pub struct DiffAdapter {
    pub from: AppManifest,
}

impl StorageAdapter for DiffAdapter {
    fn plan_schema(&self, target: &AppManifest) -> Result<SchemaPlan, StorageError> {
        let mut operations = Vec::new();

        let old_entities: std::collections::HashMap<&str, &pylon_kernel::ManifestEntity> = self
            .from
            .entities
            .iter()
            .map(|e| (e.name.as_str(), e))
            .collect();
        let new_entities: std::collections::HashMap<&str, &pylon_kernel::ManifestEntity> = target
            .entities
            .iter()
            .map(|e| (e.name.as_str(), e))
            .collect();

        // Removed entities
        for name in old_entities.keys() {
            if !new_entities.contains_key(name) {
                operations.push(SchemaOperation::RemoveEntity {
                    name: name.to_string(),
                });
            }
        }

        // Added entities
        for (name, entity) in &new_entities {
            if !old_entities.contains_key(name) {
                let fields: Vec<FieldSpec> = entity
                    .fields
                    .iter()
                    .map(|f| FieldSpec {
                        name: f.name.clone(),
                        field_type: f.field_type.clone(),
                        optional: f.optional,
                        unique: f.unique,
                    })
                    .collect();
                operations.push(SchemaOperation::CreateEntity {
                    name: name.to_string(),
                    fields,
                });
                for index in &entity.indexes {
                    operations.push(SchemaOperation::AddIndex {
                        entity: name.to_string(),
                        name: index.name.clone(),
                        fields: index.fields.clone(),
                        unique: index.unique,
                    });
                }
            }
        }

        // Field changes in shared entities
        for (name, new_entity) in &new_entities {
            if let Some(old_entity) = old_entities.get(name) {
                let old_fields: std::collections::HashSet<&str> =
                    old_entity.fields.iter().map(|f| f.name.as_str()).collect();
                let new_fields: std::collections::HashSet<&str> =
                    new_entity.fields.iter().map(|f| f.name.as_str()).collect();

                for field in &new_entity.fields {
                    if !old_fields.contains(field.name.as_str()) {
                        operations.push(SchemaOperation::AddField {
                            entity: name.to_string(),
                            field: FieldSpec {
                                name: field.name.clone(),
                                field_type: field.field_type.clone(),
                                optional: field.optional,
                                unique: field.unique,
                            },
                        });
                    }
                }

                for field in &old_entity.fields {
                    if !new_fields.contains(field.name.as_str()) {
                        operations.push(SchemaOperation::RemoveField {
                            entity: name.to_string(),
                            field_name: field.name.clone(),
                        });
                    }
                }

                // Index changes
                let old_indexes: std::collections::HashSet<&str> =
                    old_entity.indexes.iter().map(|i| i.name.as_str()).collect();
                let new_indexes: std::collections::HashSet<&str> =
                    new_entity.indexes.iter().map(|i| i.name.as_str()).collect();

                for index in &new_entity.indexes {
                    if !old_indexes.contains(index.name.as_str()) {
                        operations.push(SchemaOperation::AddIndex {
                            entity: name.to_string(),
                            name: index.name.clone(),
                            fields: index.fields.clone(),
                            unique: index.unique,
                        });
                    }
                }

                for index in &old_entity.indexes {
                    if !new_indexes.contains(index.name.as_str()) {
                        operations.push(SchemaOperation::RemoveIndex {
                            entity: name.to_string(),
                            name: index.name.clone(),
                        });
                    }
                }
            }
        }

        if operations.is_empty() {
            operations.push(SchemaOperation::Noop);
        }

        Ok(SchemaPlan { operations })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::*;

    fn minimal_manifest() -> AppManifest {
        AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "User".into(),
                fields: vec![ManifestField {
                    name: "email".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: true,
                }],
                indexes: vec![],
                relations: vec![],
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        }
    }

    #[test]
    fn dry_run_creates_all_entities() {
        let adapter = DryRunAdapter;
        let manifest = minimal_manifest();
        let plan = adapter.plan_schema(&manifest).unwrap();

        assert_eq!(plan.operations.len(), 1);
        match &plan.operations[0] {
            SchemaOperation::CreateEntity { name, fields } => {
                assert_eq!(name, "User");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].name, "email");
            }
            other => panic!("expected CreateEntity, got: {other:?}"),
        }
    }

    #[test]
    fn dry_run_includes_indexes() {
        let adapter = DryRunAdapter;
        let mut manifest = minimal_manifest();
        manifest.entities[0].indexes.push(ManifestIndex {
            name: "by_email".into(),
            fields: vec!["email".into()],
            unique: true,
        });
        let plan = adapter.plan_schema(&manifest).unwrap();

        assert_eq!(plan.operations.len(), 2);
        match &plan.operations[1] {
            SchemaOperation::AddIndex {
                entity,
                name,
                fields,
                unique,
            } => {
                assert_eq!(entity, "User");
                assert_eq!(name, "by_email");
                assert_eq!(fields, &vec!["email".to_string()]);
                assert!(unique);
            }
            other => panic!("expected AddIndex, got: {other:?}"),
        }
    }

    #[test]
    fn dry_run_empty_manifest_produces_noop() {
        let adapter = DryRunAdapter;
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "empty".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        };
        let plan = adapter.plan_schema(&manifest).unwrap();
        assert!(plan.is_empty());
    }

    #[test]
    fn diff_adapter_detects_new_entity() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.entities.push(ManifestEntity {
            name: "Post".into(),
            fields: vec![ManifestField {
                name: "title".into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
            }],
            indexes: vec![],
            relations: vec![],
        });

        let adapter = DiffAdapter { from: old };
        let plan = adapter.plan_schema(&new).unwrap();

        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::CreateEntity { name, .. } if name == "Post"
        )));
    }

    #[test]
    fn diff_adapter_detects_removed_entity() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.entities.clear();

        let adapter = DiffAdapter { from: old };
        let plan = adapter.plan_schema(&new).unwrap();

        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::RemoveEntity { name } if name == "User"
        )));
    }

    #[test]
    fn diff_adapter_detects_added_field() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.entities[0].fields.push(ManifestField {
            name: "name".into(),
            field_type: "string".into(),
            optional: false,
            unique: false,
        });

        let adapter = DiffAdapter { from: old };
        let plan = adapter.plan_schema(&new).unwrap();

        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::AddField { entity, field } if entity == "User" && field.name == "name"
        )));
    }

    #[test]
    fn diff_adapter_detects_removed_field() {
        let old = minimal_manifest();
        let mut new = minimal_manifest();
        new.entities[0].fields.clear();

        let adapter = DiffAdapter { from: old };
        let plan = adapter.plan_schema(&new).unwrap();

        assert!(plan.operations.iter().any(|op| matches!(
            op,
            SchemaOperation::RemoveField { entity, field_name } if entity == "User" && field_name == "email"
        )));
    }

    #[test]
    fn diff_adapter_no_changes_produces_noop() {
        let m = minimal_manifest();
        let adapter = DiffAdapter { from: m.clone() };
        let plan = adapter.plan_schema(&m).unwrap();
        assert!(plan.is_empty());
    }

    #[test]
    fn apply_schema_not_implemented() {
        let adapter = DryRunAdapter;
        let plan = SchemaPlan { operations: vec![] };
        let result = adapter.apply_schema(&plan);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "APPLY_NOT_IMPLEMENTED");
    }

    // -- Plan analysis tests --

    #[test]
    fn safe_plan_has_no_warnings() {
        let plan = SchemaPlan {
            operations: vec![
                SchemaOperation::CreateEntity {
                    name: "User".into(),
                    fields: vec![],
                },
                SchemaOperation::AddIndex {
                    entity: "User".into(),
                    name: "idx".into(),
                    fields: vec!["email".into()],
                    unique: true,
                },
                SchemaOperation::Noop,
            ],
        };
        let analysis = analyze_plan(&plan);
        assert!(!analysis.destructive);
        assert!(!analysis.has_unsupported);
        assert!(analysis.warnings.is_empty());
    }

    #[test]
    fn remove_entity_is_destructive() {
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::RemoveEntity {
                name: "User".into(),
            }],
        };
        let analysis = analyze_plan(&plan);
        assert!(analysis.destructive);
        assert!(analysis.has_unsupported);
        assert_eq!(analysis.warnings.len(), 1);
        assert_eq!(analysis.warnings[0].code, "DESTRUCTIVE_REMOVE_ENTITY");
    }

    #[test]
    fn remove_field_is_destructive() {
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::RemoveField {
                entity: "User".into(),
                field_name: "email".into(),
            }],
        };
        let analysis = analyze_plan(&plan);
        assert!(analysis.destructive);
        assert!(analysis.has_unsupported);
        assert_eq!(analysis.warnings[0].code, "DESTRUCTIVE_REMOVE_FIELD");
    }

    #[test]
    fn remove_index_is_unsupported_not_destructive() {
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::RemoveIndex {
                entity: "User".into(),
                name: "idx".into(),
            }],
        };
        let analysis = analyze_plan(&plan);
        assert!(!analysis.destructive);
        assert!(analysis.has_unsupported);
        assert_eq!(analysis.warnings[0].code, "UNSUPPORTED_REMOVE_INDEX");
    }

    #[test]
    fn mixed_plan_flags_both() {
        let plan = SchemaPlan {
            operations: vec![
                SchemaOperation::CreateEntity {
                    name: "Post".into(),
                    fields: vec![],
                },
                SchemaOperation::RemoveEntity {
                    name: "User".into(),
                },
                SchemaOperation::RemoveIndex {
                    entity: "Post".into(),
                    name: "idx".into(),
                },
            ],
        };
        let analysis = analyze_plan(&plan);
        assert!(analysis.destructive);
        assert!(analysis.has_unsupported);
        assert_eq!(analysis.warnings.len(), 2);
    }

    #[test]
    fn noop_plan_is_safe() {
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::Noop],
        };
        let analysis = analyze_plan(&plan);
        assert!(!analysis.destructive);
        assert!(!analysis.has_unsupported);
        assert!(analysis.warnings.is_empty());
    }

    // -- StorageError --

    #[test]
    fn storage_error_display() {
        let err = StorageError {
            code: "TEST".into(),
            message: "msg".into(),
        };
        assert_eq!(format!("{err}"), "[TEST] msg");
    }

    // -- plan_from_snapshot edge cases --

    #[test]
    fn plan_from_snapshot_empty_both() {
        let snapshot = SchemaSnapshot { tables: vec![] };
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        };
        let plan = plan_from_snapshot(&snapshot, &manifest);
        assert!(plan.is_empty());
    }

    #[test]
    fn plan_from_snapshot_add_field_to_existing() {
        let snapshot = SchemaSnapshot {
            tables: vec![TableSnapshot {
                name: "User".into(),
                columns: vec![
                    ColumnSnapshot {
                        name: "id".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: true,
                    },
                    ColumnSnapshot {
                        name: "email".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: false,
                    },
                ],
                indexes: vec![],
            }],
        };
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "User".into(),
                fields: vec![
                    ManifestField {
                        name: "email".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: true,
                    },
                    ManifestField {
                        name: "name".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                    },
                ],
                indexes: vec![],
                relations: vec![],
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        };
        let plan = plan_from_snapshot(&snapshot, &manifest);
        assert!(plan.operations.iter().any(|op| matches!(op, SchemaOperation::AddField { entity, field } if entity == "User" && field.name == "name")));
    }

    #[test]
    fn plan_from_snapshot_add_index() {
        let snapshot = SchemaSnapshot {
            tables: vec![TableSnapshot {
                name: "User".into(),
                columns: vec![
                    ColumnSnapshot {
                        name: "id".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: true,
                    },
                    ColumnSnapshot {
                        name: "email".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: false,
                    },
                ],
                indexes: vec![], // no indexes
            }],
        };
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "User".into(),
                fields: vec![ManifestField {
                    name: "email".into(),
                    field_type: "string".into(),
                    optional: false,
                    unique: true,
                }],
                indexes: vec![ManifestIndex {
                    name: "by_email".into(),
                    fields: vec!["email".into()],
                    unique: true,
                }],
                relations: vec![],
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
        };
        let plan = plan_from_snapshot(&snapshot, &manifest);
        assert!(plan
            .operations
            .iter()
            .any(|op| matches!(op, SchemaOperation::AddIndex { name, .. } if name == "by_email")));
    }

    // -- SchemaPlan::is_empty --

    #[test]
    fn plan_empty_vec_is_empty() {
        let plan = SchemaPlan { operations: vec![] };
        assert!(plan.is_empty());
    }

    #[test]
    fn plan_with_real_ops_not_empty() {
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::CreateEntity {
                name: "X".into(),
                fields: vec![],
            }],
        };
        assert!(!plan.is_empty());
    }

    // -- PlanWarning serialization --

    #[test]
    fn plan_analysis_serializable() {
        let analysis = analyze_plan(&SchemaPlan {
            operations: vec![SchemaOperation::RemoveEntity { name: "X".into() }],
        });
        let json = serde_json::to_string(&analysis).unwrap();
        assert!(json.contains("DESTRUCTIVE_REMOVE_ENTITY"));
    }
}
