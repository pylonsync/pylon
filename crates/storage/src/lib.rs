pub mod files;
#[cfg(feature = "postgres-live")]
pub mod pg_datastore;
#[cfg(feature = "postgres-live")]
pub mod pg_exec;
#[cfg(feature = "postgres-live")]
pub mod pg_search;
#[cfg(feature = "postgres-live")]
pub mod pg_tx_store;
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
    /// Change column shape on an existing field. Only nullable transitions
    /// today (`required → optional`, `optional → required`); type changes
    /// stay manual because they can fail on existing rows in ways the
    /// planner can't reason about (e.g. `string → int` on a column with
    /// non-numeric values).
    ///
    /// Real-world driver: pylon-cloud's User entity made `avatarColor`
    /// optional after the framework's OAuth handler started failing
    /// USER_CREATE_FAILED on a NOT NULL violation. Without AlterField
    /// the manifest change was a no-op against the live PG schema
    /// — operators had to drop NOT NULL by hand. Now the planner emits
    /// the ALTER and `pylon dev` auto-applies it on the next reload.
    ///
    /// `previous` carries the old column shape so the SQL emitter can
    /// decide whether to emit DROP NOT NULL, SET NOT NULL, both, or
    /// neither. `target` is the desired final shape.
    AlterField {
        entity: String,
        previous: FieldSpec,
        target: FieldSpec,
    },
    RemoveEntity {
        name: String,
    },
    AddIndex {
        entity: String,
        name: String,
        fields: Vec<String>,
        unique: bool,
        /// Optional partial-index predicate (`CREATE … WHERE <pred>`).
        /// Empty / None = full index; otherwise the SQL is appended
        /// verbatim to the CREATE INDEX. Set this for "max one row
        /// per scope" constraints that depend on a column value
        /// (e.g. `plan = 'hobby'`).
        where_clause: Option<String>,
    },
    RemoveIndex {
        entity: String,
        name: String,
    },
    /// Materialize the FTS5 + facet-bitmap shadow tables for a
    /// searchable entity. Emitted alongside CreateEntity when the
    /// manifest declares a `search:` config; idempotent so repeated
    /// pushes against a live DB are safe.
    CreateSearchIndex {
        entity: String,
        config: pylon_kernel::ManifestSearchConfig,
    },
    /// Drop an entity's search shadow tables. Emitted when the entity
    /// is removed from the manifest or its `search:` block is cleared.
    RemoveSearchIndex {
        entity: String,
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
    /// Whether the live column carries a DEFAULT expression. A NOT NULL
    /// column without one rejects inserts that omit it — the planner
    /// uses this to heal columns orphaned by manifest edits.
    pub has_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexSnapshot {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
    /// Partial-index predicate as introspected from the live DB. None
    /// means "regular index covering every row". Compared against the
    /// manifest's `where_clause` to detect shape drift — if the live
    /// index lacks a WHERE clause that the manifest declares (or
    /// vice-versa), `plan_from_snapshot` emits RemoveIndex + AddIndex
    /// to recreate it correctly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub where_clause: Option<String>,
}

/// Best-effort canonicalization of a partial-index WHERE predicate so
/// shape-drift comparison doesn't fire on cosmetic differences. The
/// only normalisation we attempt is whitespace collapsing — Postgres'
/// pg_get_expr usually adds parentheses (`(plan = 'hobby'::text)`)
/// while a manifest typically writes the bare form (`plan = 'hobby'`),
/// so we strip a single leading/trailing pair plus type-cast suffixes
/// like `::text`. This is intentionally narrow: anything fancier (sub-
/// expressions, multiple type casts, function calls) falls through and
/// produces a "drift" signal that triggers a rebuild — better to over-
/// rebuild than to silently skip a legit drift.
fn normalise_predicate(p: Option<&str>) -> Option<String> {
    p.map(|s| {
        let mut t = s.trim().to_string();
        // Strip one balanced pair of outer parens.
        if t.starts_with('(') && t.ends_with(')') {
            t = t[1..t.len() - 1].trim().to_string();
        }
        // Drop common Postgres type-cast suffixes from string
        // literals so `'hobby'::text` ≡ `'hobby'`.
        for cast in ["::text", "::varchar", "::character varying", "::bpchar"] {
            t = t.replace(cast, "");
        }
        // Collapse runs of whitespace.
        let collapsed: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
        collapsed
    })
}

/// Plan schema changes from a snapshot to a target manifest. Shared by
/// the SQLite and Postgres adapters.
///
/// Produces a mix of CreateEntity / AddField / AlterField / AddIndex /
/// RemoveIndex / Noop. RemoveIndex fires in two cases: an index in the
/// live DB has no counterpart in the manifest (someone deleted it from
/// schema and we honour the deletion), or an index exists under the
/// same name but with a drifted shape (different columns, uniqueness,
/// or partial-WHERE predicate). The drift case auto-heals indexes
/// created by an older Pylon binary that silently dropped a WHERE
/// clause — see the regression test
/// `plan_recreates_index_when_partial_predicate_drifts`.
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
                        where_clause: index.where_clause.clone(),
                    });
                }
                if let Some(cfg) = &entity.search {
                    if !cfg.is_empty() {
                        operations.push(SchemaOperation::CreateSearchIndex {
                            entity: entity.name.clone(),
                            config: cfg.clone(),
                        });
                    }
                }
            }
            Some(table) => {
                let cols_by_name: HashMap<&str, &ColumnSnapshot> =
                    table.columns.iter().map(|c| (c.name.as_str(), c)).collect();
                for field in &entity.fields {
                    match cols_by_name.get(field.name.as_str()) {
                        None => {
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
                        Some(existing) => {
                            // Detect nullable-shape drift between the live
                            // column and the manifest. Existing column's
                            // `notnull = true` ≡ manifest's `optional = false`.
                            // We DON'T re-derive type/unique here — type
                            // changes need a destructive plan we don't
                            // attempt automatically, and unique-add lives in
                            // the AddIndex path.
                            let existing_optional = !existing.notnull;
                            let target_optional = field.optional;
                            if existing_optional != target_optional {
                                let target_spec = FieldSpec {
                                    name: field.name.clone(),
                                    field_type: field.field_type.clone(),
                                    optional: target_optional,
                                    unique: field.unique,
                                };
                                let previous_spec = FieldSpec {
                                    name: field.name.clone(),
                                    field_type: existing.column_type.clone(),
                                    optional: existing_optional,
                                    unique: field.unique,
                                };
                                operations.push(SchemaOperation::AlterField {
                                    entity: entity.name.clone(),
                                    previous: previous_spec,
                                    target: target_spec,
                                });
                            }
                        }
                    }
                }
                // Columns that exist in the DB but are no longer in the
                // manifest. We never DROP them automatically (data
                // preservation), but a leftover NOT NULL column with no
                // DEFAULT rejects every future insert — the runtime
                // stopped supplying the field the moment it left the
                // manifest. Heal by relaxing to nullable: non-destructive,
                // prod-safe, and inserts work again. (Real-world driver:
                // world3d removed `isBot` and every spawn failed with
                // "NOT NULL constraint failed: Avatar.isBot".)
                let manifest_fields: HashSet<&str> =
                    entity.fields.iter().map(|f| f.name.as_str()).collect();
                for col in &table.columns {
                    if manifest_fields.contains(col.name.as_str()) {
                        continue;
                    }
                    if col.primary_key || col.name == "id" {
                        continue;
                    }
                    if col.notnull && !col.has_default {
                        operations.push(SchemaOperation::AlterField {
                            entity: entity.name.clone(),
                            previous: FieldSpec {
                                name: col.name.clone(),
                                field_type: col.column_type.clone(),
                                optional: false,
                                unique: false,
                            },
                            target: FieldSpec {
                                name: col.name.clone(),
                                field_type: col.column_type.clone(),
                                optional: true,
                                unique: false,
                            },
                        });
                    }
                }

                // Index names in DB are prefixed: {entity}_{index_name}.
                // Build a lookup keyed by the prefixed name so we can
                // detect: missing → AddIndex, drifted shape → drop +
                // recreate, removed-from-manifest → RemoveIndex.
                let existing_index_by_full: HashMap<&str, &IndexSnapshot> =
                    table.indexes.iter().map(|i| (i.name.as_str(), i)).collect();

                for index in &entity.indexes {
                    let full_name = format!("{}_{}", entity.name, index.name);
                    match existing_index_by_full.get(full_name.as_str()) {
                        None => {
                            operations.push(SchemaOperation::AddIndex {
                                entity: entity.name.clone(),
                                name: index.name.clone(),
                                fields: index.fields.clone(),
                                unique: index.unique,
                                where_clause: index.where_clause.clone(),
                            });
                        }
                        Some(existing) => {
                            // Shape drift: columns / unique / WHERE
                            // predicate differs between the live index
                            // and the manifest. Auto-heals indexes
                            // created by an older Pylon version that
                            // didn't honour the WHERE clause (shipped
                            // a regular unique index where a partial
                            // unique was requested). Drop + recreate
                            // with the manifest's intent.
                            //
                            // SQL equality on the WHERE predicate is
                            // string-comparison, which catches the
                            // common case but not semantic equivalence
                            // (`plan = 'hobby'` vs `(plan = 'hobby')`).
                            // Postgres re-renders predicates through
                            // pg_get_expr, so what we read back is the
                            // canonical form — matching it requires
                            // the manifest to use the same form. Drift
                            // in either direction triggers a rebuild.
                            let columns_match = existing.columns == index.fields;
                            let unique_match = existing.unique == index.unique;
                            let where_match = normalise_predicate(existing.where_clause.as_deref())
                                == normalise_predicate(index.where_clause.as_deref());
                            if !columns_match || !unique_match || !where_match {
                                operations.push(SchemaOperation::RemoveIndex {
                                    entity: entity.name.clone(),
                                    name: index.name.clone(),
                                });
                                operations.push(SchemaOperation::AddIndex {
                                    entity: entity.name.clone(),
                                    name: index.name.clone(),
                                    fields: index.fields.clone(),
                                    unique: index.unique,
                                    where_clause: index.where_clause.clone(),
                                });
                            }
                        }
                    }
                }

                // Indexes that exist in the DB but not in the manifest
                // — drop them. Skips the table's primary key (Postgres
                // names it `<table>_pkey`, SQLite's autoindex_*) and
                // anything whose name doesn't match the
                // `<entity>_<logical>` Pylon convention, so externally
                // managed indexes (DBA-created, search shadow tables,
                // etc.) don't get clobbered.
                let prefix = format!("{}_", entity.name);
                let manifest_index_names: HashSet<String> = entity
                    .indexes
                    .iter()
                    .map(|i| format!("{}_{}", entity.name, i.name))
                    .collect();
                for live in &table.indexes {
                    if !live.name.starts_with(&prefix) {
                        continue;
                    }
                    if live.name.ends_with("_pkey") {
                        continue;
                    }
                    if manifest_index_names.contains(&live.name) {
                        continue;
                    }
                    let logical = live.name[prefix.len()..].to_string();
                    operations.push(SchemaOperation::RemoveIndex {
                        entity: entity.name.clone(),
                        name: logical,
                    });
                }
                // Search-index creation on an already-existing table:
                // emit CreateSearchIndex when the manifest adds a
                // `search:` block to an entity that was previously
                // non-searchable. Apply is idempotent (IF NOT EXISTS
                // on the FTS + facet tables) so repeated pushes don't
                // fail; the one-way gap this closes is going from
                // "no search" → "search" on a table that already has
                // rows.
                //
                // Inverse case: the manifest used to declare `search:`
                // but no longer does (or the block was cleared). The
                // `_fts_<entity>` shadow table is still present from
                // the previous push — emit RemoveSearchIndex so the
                // apply path drops it. Without this, orphaned FTS
                // tables linger forever, waste disk, and surface
                // stale data on later queries (the maintenance loop
                // keeps writing to them if the entity is ever marked
                // searchable again).
                let search_active = entity
                    .search
                    .as_ref()
                    .map(|cfg| !cfg.is_empty())
                    .unwrap_or(false);
                if search_active {
                    if let Some(cfg) = &entity.search {
                        let fts_table = format!("_fts_{}", entity.name);
                        let facet_table = "_facet_bitmap";
                        let fts_exists = existing_tables.contains_key(fts_table.as_str());
                        let facet_exists = existing_tables.contains_key(facet_table);
                        if !fts_exists || !facet_exists {
                            operations.push(SchemaOperation::CreateSearchIndex {
                                entity: entity.name.clone(),
                                config: cfg.clone(),
                            });
                        }
                    }
                } else {
                    let fts_table = format!("_fts_{}", entity.name);
                    if existing_tables.contains_key(fts_table.as_str()) {
                        operations.push(SchemaOperation::RemoveSearchIndex {
                            entity: entity.name.clone(),
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
                // RemoveIndex is non-destructive on Postgres
                // (DROP INDEX IF EXISTS) and the apply path on the
                // SQLite adapter handles it via PRAGMA. Neither is
                // unsupported — historical version of this code
                // blanket-flagged it, which left orphan partial
                // unique indexes in production after the schema
                // dropped them. Surface a soft warning so operators
                // see the change in dry-run output, but don't block.
                warnings.push(PlanWarning {
                    code: "REMOVE_INDEX".into(),
                    message: format!(
                        "Removing index \"{}.{}\" — auto-migrate will issue DROP INDEX IF EXISTS",
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
                    where_clause: index.where_clause.clone(),
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

        // Removed entities. RemoveSearchIndex MUST precede
        // RemoveEntity in the op list: on Postgres the FTS shadow
        // table is created with a foreign-key-style reference back
        // to the parent (CASCADE on drop saves us in some paths,
        // but the deterministic order here means the apply step
        // never relies on cascade order, and on SQLite the FTS5
        // virtual table can hold a sync trigger referencing the
        // parent which must be cleared first).
        for (name, old_entity) in &old_entities {
            if !new_entities.contains_key(name) {
                let had_search = old_entity
                    .search
                    .as_ref()
                    .map(|cfg| !cfg.is_empty())
                    .unwrap_or(false);
                if had_search {
                    operations.push(SchemaOperation::RemoveSearchIndex {
                        entity: name.to_string(),
                    });
                }
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
                        where_clause: index.where_clause.clone(),
                    });
                }
                // Newly added entity with a `search:` block: mirror
                // what plan_from_snapshot does for the None-arm so
                // the diff path doesn't quietly skip search setup.
                if let Some(cfg) = &entity.search {
                    if !cfg.is_empty() {
                        operations.push(SchemaOperation::CreateSearchIndex {
                            entity: name.to_string(),
                            config: cfg.clone(),
                        });
                    }
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
                            where_clause: index.where_clause.clone(),
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

                // Search-block transitions on a shared entity.
                // - off → on: emit CreateSearchIndex so the apply
                //   path materialises the FTS shadow tables.
                // - on  → off: emit RemoveSearchIndex so the old
                //   shadow tables get dropped instead of lingering
                //   as orphans (wastes disk, returns stale data on
                //   any later read path).
                // - shape change (different text/facet/sortable
                //   columns) is intentionally NOT auto-handled
                //   here — operators have to touch the manifest
                //   twice (clear, then re-add) until we can
                //   diff the inner config safely.
                let old_search = old_entity
                    .search
                    .as_ref()
                    .map(|cfg| !cfg.is_empty())
                    .unwrap_or(false);
                let new_search = new_entity
                    .search
                    .as_ref()
                    .map(|cfg| !cfg.is_empty())
                    .unwrap_or(false);
                match (old_search, new_search) {
                    (false, true) => {
                        if let Some(cfg) = &new_entity.search {
                            operations.push(SchemaOperation::CreateSearchIndex {
                                entity: name.to_string(),
                                config: cfg.clone(),
                            });
                        }
                    }
                    (true, false) => {
                        operations.push(SchemaOperation::RemoveSearchIndex {
                            entity: name.to_string(),
                        });
                    }
                    _ => {}
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
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                }],
                indexes: vec![],
                relations: vec![],
                search: None,
                crdt: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
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
            where_clause: None,
        });
        let plan = adapter.plan_schema(&manifest).unwrap();

        assert_eq!(plan.operations.len(), 2);
        match &plan.operations[1] {
            SchemaOperation::AddIndex {
                entity,
                name,
                fields,
                unique,
                where_clause,
            } => {
                assert_eq!(entity, "User");
                assert_eq!(name, "by_email");
                assert_eq!(fields, &vec!["email".to_string()]);
                assert!(unique);
                assert!(where_clause.is_none());
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
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
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
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
            }],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
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
            crdt: None,
            server_only: false,
            readonly: false,
            default: None,
            enum_values: None,
            encrypted: false,
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
                    where_clause: None,
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
    fn remove_index_is_safe_and_supported() {
        // Reverses the previous classification. DROP INDEX IF EXISTS
        // is non-destructive on data + supported by both adapters, so
        // it should NOT block auto-migrate. analyze_plan emits a soft
        // REMOVE_INDEX warning for visibility.
        let plan = SchemaPlan {
            operations: vec![SchemaOperation::RemoveIndex {
                entity: "User".into(),
                name: "idx".into(),
            }],
        };
        let analysis = analyze_plan(&plan);
        assert!(!analysis.destructive);
        assert!(!analysis.has_unsupported);
        assert!(analysis.warnings.iter().any(|w| w.code == "REMOVE_INDEX"));
    }

    #[test]
    fn mixed_plan_flags_both() {
        // RemoveEntity is destructive + unsupported (we never auto-
        // drop tables). RemoveIndex is now safe + supported, so the
        // mix should be destructive (RemoveEntity) but neither op
        // alone is responsible for the unsupported flag from
        // RemoveIndex any more — only RemoveEntity contributes here.
        // Three warnings total: RemoveEntity (destructive +
        // unsupported = two warnings) + RemoveIndex (one).
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
        assert!(analysis
            .warnings
            .iter()
            .any(|w| w.code == "DESTRUCTIVE_REMOVE_ENTITY"));
        assert!(analysis.warnings.iter().any(|w| w.code == "REMOVE_INDEX"));
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
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
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
                        has_default: false,
                    },
                    ColumnSnapshot {
                        name: "email".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: false,
                        has_default: false,
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
                        crdt: None,
                        server_only: false,
                        readonly: false,
                        default: None,
                        enum_values: None,
                        encrypted: false,
                    },
                    ManifestField {
                        name: "name".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                        server_only: false,
                        readonly: false,
                        default: None,
                        enum_values: None,
                        encrypted: false,
                    },
                ],
                indexes: vec![],
                relations: vec![],
                search: None,
                crdt: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
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
                        has_default: false,
                    },
                    ColumnSnapshot {
                        name: "email".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: false,
                        has_default: false,
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
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                }],
                indexes: vec![ManifestIndex {
                    name: "by_email".into(),
                    fields: vec!["email".into()],
                    unique: true,
                    where_clause: None,
                }],
                relations: vec![],
                search: None,
                crdt: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let plan = plan_from_snapshot(&snapshot, &manifest);
        assert!(plan
            .operations
            .iter()
            .any(|op| matches!(op, SchemaOperation::AddIndex { name, .. } if name == "by_email")));
    }

    /// Regression for the framework bug behind pylon-cloud's
    /// `Organization_uniq_hobby_owner` constraint: an older Pylon
    /// silently created a regular unique index where the manifest
    /// declared a partial one (`WHERE plan = 'hobby'`). When a newer
    /// schema removed the index, auto-migrate kept the bad index alive
    /// because plan_from_snapshot only checked existence-by-name and
    /// never compared shape. Org creation stayed broken in prod even
    /// after the schema change shipped.
    ///
    /// Fix: detect partial-WHERE drift and emit RemoveIndex + AddIndex
    /// to recreate the index in the manifest's intended shape.
    #[test]
    fn plan_recreates_index_when_partial_predicate_drifts() {
        let snapshot = SchemaSnapshot {
            tables: vec![TableSnapshot {
                name: "Organization".into(),
                columns: vec![
                    ColumnSnapshot {
                        name: "id".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: true,
                        has_default: false,
                    },
                    ColumnSnapshot {
                        name: "createdBy".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: false,
                        has_default: false,
                    },
                    ColumnSnapshot {
                        name: "plan".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: false,
                        has_default: false,
                    },
                ],
                // Live DB has a regular unique index — no WHERE clause.
                // Mirrors the bad state pylon-cloud's prod was stuck in.
                indexes: vec![IndexSnapshot {
                    name: "Organization_uniq_hobby_owner".into(),
                    columns: vec!["createdBy".into()],
                    unique: true,
                    where_clause: None,
                }],
            }],
        };
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "Organization".into(),
                fields: vec![
                    ManifestField {
                        name: "createdBy".into(),
                        field_type: "id".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                        server_only: false,
                        readonly: false,
                        default: None,
                        enum_values: None,
                        encrypted: false,
                    },
                    ManifestField {
                        name: "plan".into(),
                        field_type: "string".into(),
                        optional: false,
                        unique: false,
                        crdt: None,
                        server_only: false,
                        readonly: false,
                        default: None,
                        enum_values: None,
                        encrypted: false,
                    },
                ],
                indexes: vec![ManifestIndex {
                    name: "uniq_hobby_owner".into(),
                    fields: vec!["createdBy".into()],
                    unique: true,
                    where_clause: Some("plan = 'hobby'".into()),
                }],
                relations: vec![],
                search: None,
                crdt: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let plan = plan_from_snapshot(&snapshot, &manifest);

        // Expect a RemoveIndex (drop the wrong-shape index) followed
        // by an AddIndex (recreate with the WHERE predicate).
        let remove_idx = plan.operations.iter().position(|op| {
            matches!(op, SchemaOperation::RemoveIndex { name, .. } if name == "uniq_hobby_owner")
        });
        let add_idx = plan.operations.iter().position(|op| {
            matches!(
                op,
                SchemaOperation::AddIndex { name, where_clause: Some(_), .. }
                    if name == "uniq_hobby_owner"
            )
        });
        assert!(
            remove_idx.is_some(),
            "expected RemoveIndex for drifted index"
        );
        assert!(add_idx.is_some(), "expected AddIndex with WHERE clause");
        assert!(
            remove_idx.unwrap() < add_idx.unwrap(),
            "remove must come before recreate"
        );
    }

    /// Regression for the second framework bug: removing an index
    /// from the schema didn't drop it from the live DB, because
    /// plan_from_snapshot only iterated over manifest indexes. The
    /// orphan index continued to enforce its constraint forever.
    #[test]
    fn plan_drops_index_removed_from_manifest() {
        let snapshot = SchemaSnapshot {
            tables: vec![TableSnapshot {
                name: "User".into(),
                columns: vec![
                    ColumnSnapshot {
                        name: "id".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: true,
                        has_default: false,
                    },
                    ColumnSnapshot {
                        name: "email".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: false,
                        has_default: false,
                    },
                ],
                indexes: vec![IndexSnapshot {
                    // DB index that the manifest no longer declares.
                    name: "User_old_idx".into(),
                    columns: vec!["email".into()],
                    unique: false,
                    where_clause: None,
                }],
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
                    unique: false,
                    crdt: None,
                    server_only: false,
                    readonly: false,
                    default: None,
                    enum_values: None,
                    encrypted: false,
                }],
                // Manifest dropped the old_idx entry.
                indexes: vec![],
                relations: vec![],
                search: None,
                crdt: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let plan = plan_from_snapshot(&snapshot, &manifest);
        assert!(
            plan.operations.iter().any(|op| matches!(
                op,
                SchemaOperation::RemoveIndex { entity, name }
                    if entity == "User" && name == "old_idx"
            )),
            "expected RemoveIndex for orphan DB index, got plan: {:?}",
            plan.operations,
        );
    }

    /// The primary-key index is always present in Postgres (named
    /// `<table>_pkey`) and never declared in the manifest. The planner
    /// must NOT try to drop it.
    #[test]
    fn plan_leaves_primary_key_alone() {
        let snapshot = SchemaSnapshot {
            tables: vec![TableSnapshot {
                name: "User".into(),
                columns: vec![ColumnSnapshot {
                    name: "id".into(),
                    column_type: "TEXT".into(),
                    notnull: true,
                    primary_key: true,
                    has_default: false,
                }],
                indexes: vec![IndexSnapshot {
                    name: "User_pkey".into(),
                    columns: vec!["id".into()],
                    unique: true,
                    where_clause: None,
                }],
            }],
        };
        let manifest = AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities: vec![ManifestEntity {
                name: "User".into(),
                fields: vec![],
                indexes: vec![],
                relations: vec![],
                search: None,
                crdt: true,
            }],
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        };
        let plan = plan_from_snapshot(&snapshot, &manifest);
        let drops_pkey = plan.operations.iter().any(|op| {
            matches!(
                op,
                SchemaOperation::RemoveIndex { name, .. } if name == "pkey"
            )
        });
        assert!(!drops_pkey, "primary key must not be dropped");
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

    // -- RemoveSearchIndex emission --
    //
    // Regression: when an entity's `search:` block is cleared from the
    // manifest but the live DB still has the `_fts_<entity>` shadow
    // table, plan_from_snapshot used to leave the orphan in place
    // forever. Now it emits RemoveSearchIndex so the apply path drops
    // the table.

    fn post_entity_no_search() -> ManifestEntity {
        ManifestEntity {
            name: "Post".into(),
            fields: vec![ManifestField {
                name: "title".into(),
                field_type: "string".into(),
                optional: false,
                unique: false,
                crdt: None,
                server_only: false,
                readonly: false,
                default: None,
                enum_values: None,
                encrypted: false,
            }],
            indexes: vec![],
            relations: vec![],
            search: None,
            crdt: true,
        }
    }

    fn manifest_with(entities: Vec<ManifestEntity>) -> AppManifest {
        AppManifest {
            manifest_version: MANIFEST_VERSION,
            name: "test".into(),
            version: "0.1.0".into(),
            entities,
            routes: vec![],
            queries: vec![],
            actions: vec![],
            policies: vec![],
            auth: Default::default(),
            llm: Default::default(),
            connections: vec![],
            crons: vec![],
            fonts: vec![],
        }
    }

    #[test]
    fn plan_emits_remove_search_index_when_search_dropped_from_manifest() {
        // Snapshot has Post table + orphan `_fts_Post` shadow table.
        // Manifest's Post entity has no `search:` block.
        let snapshot = SchemaSnapshot {
            tables: vec![
                TableSnapshot {
                    name: "Post".into(),
                    columns: vec![
                        ColumnSnapshot {
                            name: "id".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: true,
                            has_default: false,
                        },
                        ColumnSnapshot {
                            name: "title".into(),
                            column_type: "TEXT".into(),
                            notnull: true,
                            primary_key: false,
                            has_default: false,
                        },
                    ],
                    indexes: vec![],
                },
                TableSnapshot {
                    name: "_fts_Post".into(),
                    columns: vec![],
                    indexes: vec![],
                },
            ],
        };
        let manifest = manifest_with(vec![post_entity_no_search()]);
        let plan = plan_from_snapshot(&snapshot, &manifest);
        assert!(
            plan.operations.iter().any(|op| matches!(
                op,
                SchemaOperation::RemoveSearchIndex { entity } if entity == "Post"
            )),
            "expected RemoveSearchIndex for Post, got {:?}",
            plan.operations
        );
    }

    #[test]
    fn plan_does_not_emit_remove_search_index_when_no_fts_table() {
        // Manifest has no search, and the snapshot has no FTS table —
        // no RemoveSearchIndex should appear (nothing to clean up).
        let snapshot = SchemaSnapshot {
            tables: vec![TableSnapshot {
                name: "Post".into(),
                columns: vec![
                    ColumnSnapshot {
                        name: "id".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: true,
                        has_default: false,
                    },
                    ColumnSnapshot {
                        name: "title".into(),
                        column_type: "TEXT".into(),
                        notnull: true,
                        primary_key: false,
                        has_default: false,
                    },
                ],
                indexes: vec![],
            }],
        };
        let manifest = manifest_with(vec![post_entity_no_search()]);
        let plan = plan_from_snapshot(&snapshot, &manifest);
        assert!(
            !plan
                .operations
                .iter()
                .any(|op| matches!(op, SchemaOperation::RemoveSearchIndex { .. })),
            "no RemoveSearchIndex expected, got {:?}",
            plan.operations
        );
    }

    #[test]
    fn diff_adapter_emits_remove_search_index_before_remove_entity() {
        // Entity Post had search, gets dropped entirely. Plan must
        // contain RemoveSearchIndex *before* RemoveEntity so the
        // shadow tables are torn down before the parent disappears.
        let mut post = post_entity_no_search();
        post.search = Some(ManifestSearchConfig {
            text: vec!["title".into()],
            facets: vec![],
            sortable: vec![],
            language: None,
        });
        let old = manifest_with(vec![post]);
        let new = manifest_with(vec![]);
        let adapter = DiffAdapter { from: old };
        let plan = adapter.plan_schema(&new).unwrap();

        let remove_search_idx = plan.operations.iter().position(|op| {
            matches!(
                op,
                SchemaOperation::RemoveSearchIndex { entity } if entity == "Post"
            )
        });
        let remove_entity_idx = plan.operations.iter().position(|op| {
            matches!(
                op,
                SchemaOperation::RemoveEntity { name } if name == "Post"
            )
        });

        assert!(remove_search_idx.is_some(), "expected RemoveSearchIndex");
        assert!(remove_entity_idx.is_some(), "expected RemoveEntity");
        assert!(
            remove_search_idx.unwrap() < remove_entity_idx.unwrap(),
            "RemoveSearchIndex must precede RemoveEntity, got {:?}",
            plan.operations
        );
    }

    #[test]
    fn diff_adapter_emits_remove_search_index_when_search_block_cleared() {
        // Entity exists in both, but search block disappears.
        let mut old_post = post_entity_no_search();
        old_post.search = Some(ManifestSearchConfig {
            text: vec!["title".into()],
            facets: vec![],
            sortable: vec![],
            language: None,
        });
        let old = manifest_with(vec![old_post]);
        let new = manifest_with(vec![post_entity_no_search()]);
        let adapter = DiffAdapter { from: old };
        let plan = adapter.plan_schema(&new).unwrap();
        assert!(
            plan.operations.iter().any(|op| matches!(
                op,
                SchemaOperation::RemoveSearchIndex { entity } if entity == "Post"
            )),
            "expected RemoveSearchIndex, got {:?}",
            plan.operations
        );
    }
}
