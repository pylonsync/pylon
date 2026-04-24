//! Schema migration engine for pylon.
//!
//! Diffs an old manifest against a new manifest and produces a list of
//! SQL migration steps. Supports adding/removing entities, adding/removing
//! fields, and adding/removing indexes.
//!
//! Destructive operations (dropping entities, removing fields) require
//! explicit confirmation to prevent accidental data loss.

use std::collections::HashMap;

use pylon_kernel::{AppManifest, ManifestEntity, ManifestField, ManifestIndex};
use serde::Serialize;

// ---------------------------------------------------------------------------
// Migration step types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum MigrationStep {
    #[serde(rename = "create_table")]
    CreateTable { entity: String, sql: String },
    #[serde(rename = "drop_table")]
    DropTable {
        entity: String,
        sql: String,
        destructive: bool,
    },
    #[serde(rename = "add_column")]
    AddColumn {
        entity: String,
        field: String,
        sql: String,
    },
    #[serde(rename = "drop_column")]
    DropColumn {
        entity: String,
        field: String,
        sql: String,
        destructive: bool,
    },
    #[serde(rename = "create_index")]
    CreateIndex {
        entity: String,
        index: String,
        sql: String,
    },
    #[serde(rename = "drop_index")]
    DropIndex {
        entity: String,
        index: String,
        sql: String,
    },
    #[serde(rename = "rename_table")]
    RenameTable {
        from: String,
        to: String,
        sql: String,
    },
    #[serde(rename = "rename_column")]
    RenameColumn {
        entity: String,
        from: String,
        to: String,
        sql: String,
    },
}

impl MigrationStep {
    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            MigrationStep::DropTable {
                destructive: true,
                ..
            } | MigrationStep::DropColumn {
                destructive: true,
                ..
            }
        )
    }

    pub fn sql(&self) -> &str {
        match self {
            MigrationStep::CreateTable { sql, .. }
            | MigrationStep::DropTable { sql, .. }
            | MigrationStep::AddColumn { sql, .. }
            | MigrationStep::DropColumn { sql, .. }
            | MigrationStep::CreateIndex { sql, .. }
            | MigrationStep::DropIndex { sql, .. }
            | MigrationStep::RenameTable { sql, .. }
            | MigrationStep::RenameColumn { sql, .. } => sql,
        }
    }
}

// ---------------------------------------------------------------------------
// Rename hints
// ---------------------------------------------------------------------------

/// Tells the diff engine that some "drop X / add Y" pairs are actually
/// renames. Without these hints, renames look exactly like a destructive drop
/// followed by a fresh column — which is silent data loss.
#[derive(Debug, Clone, Default)]
pub struct RenameHints {
    /// Map of `old_table_name -> new_table_name`.
    pub tables: HashMap<String, String>,
    /// Map of `(table_name, old_column_name) -> new_column_name`.
    /// The table name here is the *new* name (post-rename) so column hints
    /// remain valid even when their table is also being renamed.
    pub columns: HashMap<(String, String), String>,
}

impl RenameHints {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rename_table(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.tables.insert(from.into(), to.into());
        self
    }

    pub fn rename_column(
        mut self,
        table: impl Into<String>,
        from: impl Into<String>,
        to: impl Into<String>,
    ) -> Self {
        self.columns.insert((table.into(), from.into()), to.into());
        self
    }
}

// ---------------------------------------------------------------------------
// Migration plan
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct MigrationPlan {
    pub steps: Vec<MigrationStep>,
    pub has_destructive: bool,
}

impl MigrationPlan {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub fn sql_statements(&self) -> Vec<&str> {
        self.steps.iter().map(|s| s.sql()).collect()
    }
}

// ---------------------------------------------------------------------------
// Diff engine
// ---------------------------------------------------------------------------

/// SQL dialect to emit. SQLite is the default — both backends use the same
/// DDL grammar for the operations the migrator needs, but column types differ
/// (BOOLEAN vs INTEGER for bools).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    Sqlite,
    Postgres,
}

/// Compute the migration plan to go from `old` manifest to `new` manifest.
/// Defaults to SQLite dialect.
pub fn diff(old: &AppManifest, new: &AppManifest) -> MigrationPlan {
    diff_with_renames(old, new, &RenameHints::default())
}

/// Like [`diff`] but emits SQL for the given dialect.
pub fn diff_for_dialect(old: &AppManifest, new: &AppManifest, dialect: Dialect) -> MigrationPlan {
    diff_full(old, new, &RenameHints::default(), dialect)
}

/// Like [`diff`], but treats `(old_name, new_name)` pairs from `hints` as
/// renames instead of drop+add. Use this when you intentionally renamed an
/// entity or field and don't want the diff engine to drop the old data.
pub fn diff_with_renames(
    old: &AppManifest,
    new: &AppManifest,
    hints: &RenameHints,
) -> MigrationPlan {
    diff_full(old, new, hints, Dialect::Sqlite)
}

/// Full-featured diff that takes both renames and a dialect.
pub fn diff_full(
    old: &AppManifest,
    new: &AppManifest,
    hints: &RenameHints,
    dialect: Dialect,
) -> MigrationPlan {
    let mut steps = Vec::new();

    let old_entities: std::collections::HashMap<&str, &ManifestEntity> =
        old.entities.iter().map(|e| (e.name.as_str(), e)).collect();
    let new_entities: std::collections::HashMap<&str, &ManifestEntity> =
        new.entities.iter().map(|e| (e.name.as_str(), e)).collect();

    // Apply table renames first so subsequent column diffs see the new name.
    let mut renamed_old_to_new: HashMap<String, String> = HashMap::new();
    for (from, to) in &hints.tables {
        if old_entities.contains_key(from.as_str()) && new_entities.contains_key(to.as_str()) {
            steps.push(MigrationStep::RenameTable {
                from: from.clone(),
                to: to.clone(),
                sql: format!(
                    "ALTER TABLE {} RENAME TO {}",
                    quote_ident(from),
                    quote_ident(to),
                ),
            });
            renamed_old_to_new.insert(from.clone(), to.clone());
        }
    }

    // New entities (CREATE TABLE) — skip ones that were renamed in.
    let renamed_to: std::collections::HashSet<&str> =
        renamed_old_to_new.values().map(|s| s.as_str()).collect();
    for (name, entity) in &new_entities {
        if !old_entities.contains_key(name) && !renamed_to.contains(name) {
            steps.push(MigrationStep::CreateTable {
                entity: name.to_string(),
                sql: create_table_sql(entity, dialect),
            });
            for idx in &entity.indexes {
                steps.push(MigrationStep::CreateIndex {
                    entity: name.to_string(),
                    index: idx.name.clone(),
                    sql: create_index_sql(name, idx),
                });
            }
        }
    }

    // Removed entities (DROP TABLE) — skip ones that were renamed away.
    for (name, _) in &old_entities {
        if !new_entities.contains_key(name) && !renamed_old_to_new.contains_key(*name) {
            steps.push(MigrationStep::DropTable {
                entity: name.to_string(),
                sql: format!("DROP TABLE IF EXISTS {}", quote_ident(name)),
                destructive: true,
            });
        }
    }

    // Modified entities (ADD/DROP columns and indexes).
    // For renamed tables, walk the old entity under its NEW name so column
    // rename hints (which key on the new table name) match.
    for (name, new_entity) in &new_entities {
        let old_entity = if let Some(e) = old_entities.get(name) {
            Some(*e)
        } else {
            // Find via reverse rename map.
            renamed_old_to_new
                .iter()
                .find(|(_, to)| to.as_str() == *name)
                .and_then(|(from, _)| old_entities.get(from.as_str()).copied())
        };
        if let Some(old_entity) = old_entity {
            diff_entity(name, old_entity, new_entity, hints, dialect, &mut steps);
        }
    }

    let has_destructive = steps.iter().any(|s| s.is_destructive());
    MigrationPlan {
        steps,
        has_destructive,
    }
}

fn diff_entity(
    entity_name: &str,
    old: &ManifestEntity,
    new: &ManifestEntity,
    hints: &RenameHints,
    dialect: Dialect,
    steps: &mut Vec<MigrationStep>,
) {
    let old_fields: std::collections::HashSet<&str> =
        old.fields.iter().map(|f| f.name.as_str()).collect();
    let new_fields: std::collections::HashSet<&str> =
        new.fields.iter().map(|f| f.name.as_str()).collect();

    // Apply column renames first.
    let mut renamed_old_field: HashMap<String, String> = HashMap::new();
    for ((tbl, from), to) in &hints.columns {
        if tbl == entity_name
            && old_fields.contains(from.as_str())
            && new_fields.contains(to.as_str())
        {
            steps.push(MigrationStep::RenameColumn {
                entity: entity_name.to_string(),
                from: from.clone(),
                to: to.clone(),
                sql: format!(
                    "ALTER TABLE {} RENAME COLUMN {} TO {}",
                    quote_ident(entity_name),
                    quote_ident(from),
                    quote_ident(to),
                ),
            });
            renamed_old_field.insert(from.clone(), to.clone());
        }
    }

    let renamed_to_set: std::collections::HashSet<&str> =
        renamed_old_field.values().map(|s| s.as_str()).collect();

    // New fields (ADD COLUMN) — skip renamed-in.
    for field in &new.fields {
        if !old_fields.contains(field.name.as_str())
            && !renamed_to_set.contains(field.name.as_str())
        {
            steps.push(MigrationStep::AddColumn {
                entity: entity_name.to_string(),
                field: field.name.clone(),
                sql: add_column_sql(entity_name, field, dialect),
            });
        }
    }

    // Removed fields (DROP COLUMN) — skip renamed-away.
    for field in &old.fields {
        if !new_fields.contains(field.name.as_str()) && !renamed_old_field.contains_key(&field.name)
        {
            steps.push(MigrationStep::DropColumn {
                entity: entity_name.to_string(),
                field: field.name.clone(),
                sql: format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    quote_ident(entity_name),
                    quote_ident(&field.name)
                ),
                destructive: true,
            });
        }
    }

    // Index changes.
    let old_indexes: std::collections::HashSet<&str> =
        old.indexes.iter().map(|i| i.name.as_str()).collect();
    let new_indexes: std::collections::HashSet<&str> =
        new.indexes.iter().map(|i| i.name.as_str()).collect();

    for idx in &new.indexes {
        if !old_indexes.contains(idx.name.as_str()) {
            steps.push(MigrationStep::CreateIndex {
                entity: entity_name.to_string(),
                index: idx.name.clone(),
                sql: create_index_sql(entity_name, idx),
            });
        }
    }

    for idx in &old.indexes {
        if !new_indexes.contains(idx.name.as_str()) {
            steps.push(MigrationStep::DropIndex {
                entity: entity_name.to_string(),
                index: idx.name.clone(),
                sql: format!("DROP INDEX IF EXISTS {}", quote_ident(&idx.name)),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// SQL generation
// ---------------------------------------------------------------------------

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn sql_type(field_type: &str, dialect: Dialect) -> &'static str {
    match (field_type, dialect) {
        ("int", _) => "INTEGER",
        ("float", Dialect::Sqlite) => "REAL",
        ("float", Dialect::Postgres) => "DOUBLE PRECISION",
        // SQLite has no real bool — store as INTEGER. Postgres has BOOLEAN.
        ("bool", Dialect::Sqlite) => "INTEGER",
        ("bool", Dialect::Postgres) => "BOOLEAN",
        ("datetime", Dialect::Postgres) => "TIMESTAMPTZ",
        _ => "TEXT",
    }
}

fn create_table_sql(entity: &ManifestEntity, dialect: Dialect) -> String {
    let mut cols = vec!["\"id\" TEXT PRIMARY KEY NOT NULL".to_string()];

    for field in &entity.fields {
        let col_type = sql_type(&field.field_type, dialect);
        let not_null = if field.optional { "" } else { " NOT NULL" };
        let unique = if field.unique { " UNIQUE" } else { "" };
        cols.push(format!(
            "{} {col_type}{not_null}{unique}",
            quote_ident(&field.name)
        ));
    }

    format!(
        "CREATE TABLE IF NOT EXISTS {} ({})",
        quote_ident(&entity.name),
        cols.join(", ")
    )
}

fn add_column_sql(entity_name: &str, field: &ManifestField, dialect: Dialect) -> String {
    let col_type = sql_type(&field.field_type, dialect);
    // New columns must be nullable or have a default in both dialects.
    // We add them as nullable regardless of schema to avoid breaking existing
    // rows, then rely on application-level validation.
    format!(
        "ALTER TABLE {} ADD COLUMN {} {}",
        quote_ident(entity_name),
        quote_ident(&field.name),
        col_type,
    )
}

fn create_index_sql(entity_name: &str, idx: &ManifestIndex) -> String {
    let unique_kw = if idx.unique { "UNIQUE " } else { "" };
    let quoted_fields: Vec<String> = idx.fields.iter().map(|f| quote_ident(f)).collect();
    format!(
        "CREATE {unique_kw}INDEX IF NOT EXISTS {} ON {} ({})",
        quote_ident(&idx.name),
        quote_ident(entity_name),
        quoted_fields.join(", ")
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use pylon_kernel::*;

    fn manifest(entities: Vec<ManifestEntity>) -> AppManifest {
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

    fn entity(
        name: &str,
        fields: Vec<ManifestField>,
        indexes: Vec<ManifestIndex>,
    ) -> ManifestEntity {
        ManifestEntity {
            name: name.into(),
            fields,
            indexes,
            relations: vec![],
            search: None,
        }
    }

    fn field(name: &str, ft: &str) -> ManifestField {
        ManifestField {
            name: name.into(),
            field_type: ft.into(),
            optional: false,
            unique: false,
        }
    }

    fn index(name: &str, fields: Vec<&str>, unique: bool) -> ManifestIndex {
        ManifestIndex {
            name: name.into(),
            fields: fields.into_iter().map(|f| f.into()).collect(),
            unique,
        }
    }

    #[test]
    fn no_changes_produces_empty_plan() {
        let m = manifest(vec![entity("User", vec![field("email", "string")], vec![])]);
        let plan = diff(&m, &m);
        assert!(plan.is_empty());
        assert!(!plan.has_destructive);
    }

    #[test]
    fn new_entity_creates_table() {
        let old = manifest(vec![]);
        let new = manifest(vec![entity("User", vec![field("email", "string")], vec![])]);
        let plan = diff(&old, &new);
        assert_eq!(plan.steps.len(), 1);
        assert!(
            matches!(&plan.steps[0], MigrationStep::CreateTable { entity, .. } if entity == "User")
        );
        assert!(!plan.has_destructive);
    }

    #[test]
    fn removed_entity_drops_table() {
        let old = manifest(vec![entity("User", vec![field("email", "string")], vec![])]);
        let new = manifest(vec![]);
        let plan = diff(&old, &new);
        assert_eq!(plan.steps.len(), 1);
        assert!(
            matches!(&plan.steps[0], MigrationStep::DropTable { entity, destructive: true, .. } if entity == "User")
        );
        assert!(plan.has_destructive);
    }

    #[test]
    fn new_field_adds_column() {
        let old = manifest(vec![entity("User", vec![field("email", "string")], vec![])]);
        let new = manifest(vec![entity(
            "User",
            vec![field("email", "string"), field("name", "string")],
            vec![],
        )]);
        let plan = diff(&old, &new);
        assert_eq!(plan.steps.len(), 1);
        assert!(
            matches!(&plan.steps[0], MigrationStep::AddColumn { entity, field, .. } if entity == "User" && field == "name")
        );
    }

    #[test]
    fn removed_field_drops_column() {
        let old = manifest(vec![entity(
            "User",
            vec![field("email", "string"), field("name", "string")],
            vec![],
        )]);
        let new = manifest(vec![entity("User", vec![field("email", "string")], vec![])]);
        let plan = diff(&old, &new);
        assert_eq!(plan.steps.len(), 1);
        assert!(matches!(
            &plan.steps[0],
            MigrationStep::DropColumn {
                destructive: true,
                ..
            }
        ));
        assert!(plan.has_destructive);
    }

    #[test]
    fn new_index_creates_index() {
        let old = manifest(vec![entity("User", vec![field("email", "string")], vec![])]);
        let new = manifest(vec![entity(
            "User",
            vec![field("email", "string")],
            vec![index("idx_email", vec!["email"], true)],
        )]);
        let plan = diff(&old, &new);
        assert_eq!(plan.steps.len(), 1);
        assert!(
            matches!(&plan.steps[0], MigrationStep::CreateIndex { index, .. } if index == "idx_email")
        );
    }

    #[test]
    fn removed_index_drops_index() {
        let old = manifest(vec![entity(
            "User",
            vec![field("email", "string")],
            vec![index("idx_email", vec!["email"], true)],
        )]);
        let new = manifest(vec![entity("User", vec![field("email", "string")], vec![])]);
        let plan = diff(&old, &new);
        assert_eq!(plan.steps.len(), 1);
        assert!(
            matches!(&plan.steps[0], MigrationStep::DropIndex { index, .. } if index == "idx_email")
        );
    }

    #[test]
    fn complex_migration() {
        let old = manifest(vec![
            entity(
                "User",
                vec![field("email", "string"), field("age", "int")],
                vec![],
            ),
            entity("Post", vec![field("title", "string")], vec![]),
        ]);
        let new = manifest(vec![
            entity(
                "User",
                vec![field("email", "string"), field("name", "string")],
                vec![index("idx_email", vec!["email"], true)],
            ),
            entity("Comment", vec![field("body", "string")], vec![]),
        ]);

        let plan = diff(&old, &new);

        // Should have: create Comment, drop Post, add name column, drop age column, create index
        assert!(plan.has_destructive);
        let step_types: Vec<&str> = plan
            .steps
            .iter()
            .map(|s| match s {
                MigrationStep::CreateTable { .. } => "create_table",
                MigrationStep::DropTable { .. } => "drop_table",
                MigrationStep::AddColumn { .. } => "add_column",
                MigrationStep::DropColumn { .. } => "drop_column",
                MigrationStep::CreateIndex { .. } => "create_index",
                MigrationStep::DropIndex { .. } => "drop_index",
                MigrationStep::RenameTable { .. } => "rename_table",
                MigrationStep::RenameColumn { .. } => "rename_column",
            })
            .collect();

        assert!(step_types.contains(&"create_table"));
        assert!(step_types.contains(&"drop_table"));
        assert!(step_types.contains(&"add_column"));
        assert!(step_types.contains(&"drop_column"));
        assert!(step_types.contains(&"create_index"));
    }

    #[test]
    fn table_rename_with_hint_emits_rename_step_not_drop() {
        let old = manifest(vec![entity("Post", vec![field("title", "string")], vec![])]);
        let new = manifest(vec![entity(
            "Article",
            vec![field("title", "string")],
            vec![],
        )]);
        let hints = RenameHints::new().rename_table("Post", "Article");
        let plan = diff_with_renames(&old, &new, &hints);
        assert!(!plan.has_destructive, "rename must not be destructive");
        let kinds: Vec<&str> = plan
            .steps
            .iter()
            .map(|s| match s {
                MigrationStep::RenameTable { .. } => "rename",
                MigrationStep::CreateTable { .. } => "create",
                MigrationStep::DropTable { .. } => "drop",
                _ => "other",
            })
            .collect();
        assert!(kinds.contains(&"rename"));
        assert!(!kinds.contains(&"create"));
        assert!(!kinds.contains(&"drop"));
    }

    #[test]
    fn column_rename_with_hint_emits_rename_step_not_drop() {
        let old = manifest(vec![entity("User", vec![field("name", "string")], vec![])]);
        let new = manifest(vec![entity(
            "User",
            vec![field("displayName", "string")],
            vec![],
        )]);
        let hints = RenameHints::new().rename_column("User", "name", "displayName");
        let plan = diff_with_renames(&old, &new, &hints);
        assert!(!plan.has_destructive);
        assert!(matches!(
            &plan.steps[0],
            MigrationStep::RenameColumn { from, to, .. } if from == "name" && to == "displayName"
        ));
    }

    #[test]
    fn rename_table_then_rename_column_inside_it() {
        let old = manifest(vec![entity("Post", vec![field("title", "string")], vec![])]);
        let new = manifest(vec![entity(
            "Article",
            vec![field("headline", "string")],
            vec![],
        )]);
        let hints = RenameHints::new()
            .rename_table("Post", "Article")
            .rename_column("Article", "title", "headline");
        let plan = diff_with_renames(&old, &new, &hints);
        assert!(!plan.has_destructive);
        let sql_concat: String = plan.sql_statements().join(";");
        assert!(sql_concat.contains("RENAME TO \"Article\""));
        assert!(sql_concat.contains("RENAME COLUMN \"title\" TO \"headline\""));
    }

    #[test]
    fn rename_hint_for_nonexistent_table_is_ignored() {
        let old = manifest(vec![entity("Post", vec![field("title", "string")], vec![])]);
        let new = manifest(vec![entity("Post", vec![field("title", "string")], vec![])]);
        let hints = RenameHints::new().rename_table("Ghost", "Spirit");
        let plan = diff_with_renames(&old, &new, &hints);
        assert!(plan.is_empty());
    }

    #[test]
    fn diff_without_hint_treats_rename_as_drop_plus_add() {
        // Sanity check that the old behavior is preserved when no hints are
        // supplied — it MUST stay destructive so users get warned.
        let old = manifest(vec![entity("Post", vec![field("title", "string")], vec![])]);
        let new = manifest(vec![entity(
            "Article",
            vec![field("title", "string")],
            vec![],
        )]);
        let plan = diff(&old, &new);
        assert!(plan.has_destructive);
    }

    #[test]
    fn postgres_dialect_uses_boolean_and_timestamptz() {
        let m_old = manifest(vec![]);
        let m_new = manifest(vec![entity(
            "Event",
            vec![
                field("ok", "bool"),
                field("at", "datetime"),
                field("count", "int"),
                field("ratio", "float"),
            ],
            vec![],
        )]);
        let plan_pg = diff_for_dialect(&m_old, &m_new, Dialect::Postgres);
        let sql_pg = plan_pg.sql_statements().join(" ");
        assert!(sql_pg.contains("BOOLEAN"));
        assert!(sql_pg.contains("TIMESTAMPTZ"));
        assert!(sql_pg.contains("INTEGER"));
        assert!(sql_pg.contains("DOUBLE PRECISION"));
        assert!(
            !sql_pg.contains("REAL"),
            "Postgres should NOT use SQLite's REAL"
        );

        let plan_sqlite = diff_for_dialect(&m_old, &m_new, Dialect::Sqlite);
        let sql_sqlite = plan_sqlite.sql_statements().join(" ");
        assert!(sql_sqlite.contains("REAL"));
        // SQLite has no native bool — booleans land as INTEGER.
        assert!(!sql_sqlite.contains("BOOLEAN"));
        assert!(!sql_sqlite.contains("TIMESTAMPTZ"));
    }

    #[test]
    fn postgres_add_column_uses_boolean() {
        let old = manifest(vec![entity("U", vec![field("name", "string")], vec![])]);
        let new = manifest(vec![entity(
            "U",
            vec![field("name", "string"), field("active", "bool")],
            vec![],
        )]);
        let plan = diff_for_dialect(&old, &new, Dialect::Postgres);
        let sql = plan.sql_statements().join(" ");
        assert!(sql.contains("ADD COLUMN \"active\" BOOLEAN"));
    }

    #[test]
    fn sql_statements_returns_all_sql() {
        let old = manifest(vec![]);
        let new = manifest(vec![entity("User", vec![field("email", "string")], vec![])]);
        let plan = diff(&old, &new);
        let sql = plan.sql_statements();
        assert_eq!(sql.len(), 1);
        assert!(sql[0].starts_with("CREATE TABLE"));
    }
}
