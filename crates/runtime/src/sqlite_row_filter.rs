//! SQL for a [`RowFilter`] on SQLite: the `WHERE` part a scan adds so it
//! reads only the rows that can pass the caller's own row check.
//!
//! Each condition holds exactly when the row check, reading the row as
//! `row_to_json` returns it, would see it hold. SQLite keeps a value's own
//! storage class, so each condition checks it (`typeof`): `row_to_json`
//! reads a blob or an infinite real as null, and an integer in a `bool`
//! column as a bool. A `datetime` is text in whatever form it was
//! written, so time comparisons are left to the row check.

use std::collections::HashMap;

use pylon_kernel::row_filter::{
    CompareAs, CompareOp, ExistsValue, RenderedFilter, RowFilter, SqlDialect,
};
use pylon_kernel::ManifestEntity;
use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use crate::quote_ident;

/// SQL for `filter` over the row aliased `t` of `entity`. Values go in
/// `params`, numbered from `first_param`.
pub(crate) fn render(
    entity: &str,
    filter: &RowFilter,
    entities: &HashMap<String, ManifestEntity>,
    first_param: usize,
    params: &mut Vec<SqlValue>,
) -> RenderedFilter {
    filter.to_sql(&mut SqliteDialect {
        entity,
        entities,
        first_param,
        params,
    })
}

struct SqliteDialect<'a> {
    entity: &'a str,
    entities: &'a HashMap<String, ManifestEntity>,
    first_param: usize,
    params: &'a mut Vec<SqlValue>,
}

/// How a column's values read, by its declared field type.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Text,
    Int,
    Float,
    Bool,
    /// Text the row check reads as written (`datetime`).
    OtherText,
}

impl SqliteDialect<'_> {
    fn kind(&self, entity: &str, column: &str) -> Option<Kind> {
        if column == "id" {
            return Some(Kind::Text);
        }
        let field = self
            .entities
            .get(entity)?
            .fields
            .iter()
            .find(|f| f.name == column)?;
        if field.encrypted {
            return None;
        }
        Some(match field.field_type.as_str() {
            "string" | "richtext" => Kind::Text,
            t if t.starts_with("id(") => Kind::Text,
            "int" => Kind::Int,
            "float" => Kind::Float,
            "bool" => Kind::Bool,
            "datetime" => Kind::OtherText,
            _ => return None,
        })
    }

    fn bind(&mut self, value: SqlValue) -> String {
        self.params.push(value);
        format!("?{}", self.first_param + self.params.len() - 1)
    }
}

/// A condition that holds when `column` reads as a value of `kind`, not
/// null.
fn present(column: &str, kind: Kind) -> String {
    match kind {
        Kind::Text | Kind::OtherText => format!("typeof({column}) = 'text'"),
        Kind::Bool => format!("typeof({column}) = 'integer'"),
        // `9e999` is infinity.
        Kind::Int | Kind::Float => format!(
            "(typeof({column}) = 'integer' OR (typeof({column}) = 'real' AND abs({column}) < 9e999))"
        ),
    }
}

impl SqlDialect for SqliteDialect<'_> {
    fn compare(
        &mut self,
        column: &str,
        op: CompareOp,
        value: &Value,
        compare_as: CompareAs,
        negated: bool,
    ) -> Option<String> {
        let kind = self.kind(self.entity, column)?;
        let col = format!("t.{}", quote_ident(column));
        let param = match (compare_as, kind) {
            (CompareAs::Text, Kind::Text) => SqlValue::Text(value.as_str()?.to_string()),
            (CompareAs::Number, Kind::Int | Kind::Float) => SqlValue::Real(value.as_f64()?),
            (CompareAs::Bool, Kind::Bool) => SqlValue::Integer(value.as_bool()? as i64),
            _ => return None,
        };
        let (op, param) = match op {
            // A bool reads as `true` for any integer but 0.
            CompareOp::Eq if param == SqlValue::Integer(1) && kind == Kind::Bool => {
                ("<>", SqlValue::Integer(0))
            }
            CompareOp::Eq => ("=", param),
            CompareOp::Lt => ("<", param),
            CompareOp::Lte => ("<=", param),
            CompareOp::Gt => (">", param),
            CompareOp::Gte => (">=", param),
        };
        let param = self.bind(param);
        // Never `NULL`: the storage check comes first and is never null.
        let condition = format!("({} AND {col} {op} {param})", present(&col, kind));
        // The row check reads text in a number column as a number when it
        // parses as one (` 5`, `inf`). Such rows are kept, and a `NOT`
        // above drops them.
        Some(match kind {
            Kind::Int | Kind::Float if !negated => {
                format!("({condition} OR typeof({col}) = 'text')")
            }
            _ => condition,
        })
    }

    fn is_null(&mut self, column: &str) -> Option<String> {
        self.kind(self.entity, column)?;
        let col = format!("t.{}", quote_ident(column));
        Some(format!(
            "(typeof({col}) IN ('null', 'blob') OR (typeof({col}) = 'real' AND NOT abs({col}) < 9e999))"
        ))
    }

    fn exists(&mut self, other: &str, conditions: &[(String, ExistsValue)]) -> Option<String> {
        // Every condition is checked before any value is bound.
        for (field, value) in conditions {
            if self.kind(other, field)? != Kind::Text {
                return None;
            }
            match value {
                ExistsValue::Value(value) => {
                    value.as_str()?;
                }
                ExistsValue::Column(column) => {
                    if self.kind(self.entity, column)? != Kind::Text {
                        return None;
                    }
                }
            }
        }
        let mut clauses = Vec::with_capacity(conditions.len());
        for (field, value) in conditions {
            let rhs = match value {
                ExistsValue::Value(value) => {
                    self.bind(SqlValue::Text(value.as_str().unwrap_or_default().into()))
                }
                ExistsValue::Column(column) => format!("t.{}", quote_ident(column)),
            };
            clauses.push(format!("x.{} = {rhs}", quote_ident(field)));
        }
        let condition = if clauses.is_empty() {
            "1".to_string()
        } else {
            clauses.join(" AND ")
        };
        Some(format!(
            "EXISTS (SELECT 1 FROM {} x WHERE {condition})",
            quote_ident(other)
        ))
    }
}
