//! SQL for a [`RowFilter`] on Postgres: the `WHERE` part a scan adds so it
//! reads only the rows that can pass the caller's own row check.
//!
//! Each condition holds exactly when the row check, reading the row as
//! [`crate::postgres::row_to_json_pub`] returns it, would see it hold. That
//! decoder returns null for a `NaN` or infinite `double precision` and for
//! an infinite `timestamptz`, so those count as null here too.

#![cfg(feature = "postgres-live")]

use std::collections::HashMap;

use pylon_kernel::row_filter::{
    CompareAs, CompareOp, ExistsValue, RenderedFilter, RowFilter, SqlDialect,
};
use serde_json::Value;

use crate::postgres::{quote_ident_pub as quote_ident, JsonParam};

/// Column types as Postgres spells them (`format_type`), per table.
pub type ColumnTypes = HashMap<String, HashMap<String, String>>;

/// The column types of `tables`, read in one statement.
pub fn column_types<C: crate::pg_exec::PgConn>(
    conn: &mut C,
    tables: &[&str],
) -> Result<ColumnTypes, postgres::Error> {
    let quoted: Vec<String> = tables.iter().map(|t| quote_ident(t)).collect();
    let rows = conn.query(
        "SELECT t.name, a.attname::text, format_type(a.atttypid, a.atttypmod) \
         FROM unnest($1::text[], $2::text[]) AS t(name, quoted) \
         JOIN pg_attribute a ON a.attrelid = to_regclass(t.quoted) \
         WHERE a.attnum > 0 AND NOT a.attisdropped",
        &[&tables, &quoted],
    )?;
    let mut types = ColumnTypes::new();
    for row in rows {
        types
            .entry(row.get::<_, String>(0))
            .or_default()
            .insert(row.get(1), row.get(2));
    }
    Ok(types)
}

/// The tables a filter reads: `entity` and every `exists` entity.
pub fn tables<'a>(entity: &'a str, filter: &'a RowFilter, out: &mut Vec<&'a str>) {
    if !out.contains(&entity) {
        out.push(entity);
    }
    match filter {
        RowFilter::And(parts) | RowFilter::Or(parts) => {
            for part in parts {
                tables(entity, part, out);
            }
        }
        RowFilter::Not(inner) => tables(entity, inner, out),
        RowFilter::Exists { entity: other, .. } if !out.contains(&other.as_str()) => {
            out.push(other);
        }
        _ => {}
    }
}

/// SQL for `filter` over the row aliased `t` of `entity`. Values go in
/// `params`, numbered from `first_param`. A part on a column the table
/// lacks, or of a type the filter does not compare it as, keeps more rows
/// and makes the result not [`RenderedFilter::exact`].
pub fn render(
    entity: &str,
    filter: &RowFilter,
    types: &ColumnTypes,
    first_param: usize,
    params: &mut Vec<JsonParam>,
) -> RenderedFilter {
    filter.to_sql(&mut PgDialect {
        entity,
        types,
        first_param,
        params,
    })
}

struct PgDialect<'a> {
    entity: &'a str,
    types: &'a ColumnTypes,
    first_param: usize,
    params: &'a mut Vec<JsonParam>,
}

impl PgDialect<'_> {
    fn column_type(&self, table: &str, column: &str) -> Option<String> {
        self.types.get(table)?.get(column).cloned()
    }

    fn bind(&mut self, value: JsonParam) -> String {
        self.params.push(value);
        format!("${}", self.first_param + self.params.len() - 1)
    }
}

fn is_text(ty: &str) -> bool {
    ty == "text" || ty.starts_with("character varying")
}

/// A condition that holds when `column` (of type `ty`) is not one of the
/// values the decoder reads as null; `None` when there are none.
fn finite(column: &str, ty: &str) -> Option<String> {
    match ty {
        "double precision" | "real" => Some(format!(
            "({column} > '-Infinity'::float8 AND {column} < 'Infinity'::float8)"
        )),
        "timestamp with time zone" => Some(format!("isfinite({column})")),
        _ => None,
    }
}

impl SqlDialect for PgDialect<'_> {
    fn compare(
        &mut self,
        column: &str,
        op: CompareOp,
        value: &Value,
        compare_as: CompareAs,
        negated: bool,
    ) -> Option<String> {
        let ty = self.column_type(self.entity, column)?;
        let param = match compare_as {
            CompareAs::Text if is_text(&ty) => {
                let text = value.as_str()?.to_string();
                format!("{}::text", self.bind(JsonParam::Text(text)))
            }
            CompareAs::Number => {
                let n = value.as_f64()?;
                match ty.as_str() {
                    // An integral bound binds as the column's own type, so
                    // its index applies.
                    "bigint" | "integer" | "smallint" if n.fract() == 0.0 && n.abs() < 9e15 => {
                        format!("{}::int8", self.bind(JsonParam::Int(n as i64)))
                    }
                    "bigint" | "integer" | "smallint" | "double precision" | "real" => {
                        format!("{}::float8", self.bind(JsonParam::Float(n)))
                    }
                    _ => return None,
                }
            }
            CompareAs::Bool if ty == "boolean" => {
                format!("{}::boolean", self.bind(JsonParam::Bool(value.as_bool()?)))
            }
            CompareAs::Time if ty == "timestamp with time zone" => {
                let at = chrono::DateTime::parse_from_rfc3339(value.as_str()?).ok()?;
                let at = at.with_timezone(&chrono::Utc).to_rfc3339();
                format!("{}::timestamptz", self.bind(JsonParam::Text(at)))
            }
            _ => return None,
        };
        let op = match op {
            CompareOp::Eq => "=",
            CompareOp::Lt => "<",
            CompareOp::Lte => "<=",
            CompareOp::Gt => ">",
            CompareOp::Gte => ">=",
        };
        let col = format!("t.{}", quote_ident(column));
        let mut condition = format!("{col} {op} {param}");
        // A bound is finite, so only an ordering can hold for a value the
        // decoder reads as null.
        if op != "=" {
            if let Some(finite) = finite(&col, &ty) {
                condition = format!("{condition} AND {finite}");
            }
        }
        // A `WHERE` drops `NULL`, but a `NOT` above would turn it true. The
        // plain form keeps the column's index usable.
        Some(if negated {
            format!("COALESCE({condition}, FALSE)")
        } else {
            format!("({condition})")
        })
    }

    fn is_null(&mut self, column: &str) -> Option<String> {
        let ty = self.column_type(self.entity, column)?;
        let col = format!("t.{}", quote_ident(column));
        Some(match finite(&col, &ty) {
            Some(finite) => format!("({col} IS NULL OR NOT {finite})"),
            None => format!("({col} IS NULL)"),
        })
    }

    fn exists(&mut self, other: &str, conditions: &[(String, ExistsValue)]) -> Option<String> {
        // Every condition is checked before any value is bound: a bound
        // value no condition reads fails the statement.
        for (field, value) in conditions {
            if !is_text(&self.column_type(other, field)?) {
                return None;
            }
            match value {
                ExistsValue::Value(value) => {
                    value.as_str()?;
                }
                ExistsValue::Column(column) => {
                    if !is_text(&self.column_type(self.entity, column)?) {
                        return None;
                    }
                }
            }
        }
        let mut clauses = Vec::with_capacity(conditions.len());
        for (field, value) in conditions {
            let rhs = match value {
                ExistsValue::Value(value) => {
                    let text = value.as_str().unwrap_or_default().to_string();
                    format!("{}::text", self.bind(JsonParam::Text(text)))
                }
                ExistsValue::Column(column) => format!("t.{}", quote_ident(column)),
            };
            clauses.push(format!("x.{} = {rhs}", quote_ident(field)));
        }
        let condition = if clauses.is_empty() {
            "TRUE".to_string()
        } else {
            clauses.join(" AND ")
        };
        Some(format!(
            "EXISTS (SELECT 1 FROM {} x WHERE {condition})",
            quote_ident(other)
        ))
    }
}
