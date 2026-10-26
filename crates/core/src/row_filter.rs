//! A predicate over one entity row that a store can run in its query, so a
//! scan reads the rows that can pass instead of every row.
//!
//! The policy engine builds one from a sync scope with the caller's values
//! filled in (`PolicyEngine::sync_scope_filter`). It keeps every row the
//! scope allows and may keep more: the caller still checks each row it
//! reads, and the filter only spares it reading the rest. Each store writes
//! it as SQL through a [`SqlDialect`].

use serde_json::Value;

/// How a [`RowFilter::Compare`] compares a column with a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareOp {
    Eq,
    Lt,
    Lte,
    Gt,
    Gte,
}

/// The type a comparison reads its column and value as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompareAs {
    Text,
    Number,
    Bool,
    /// An RFC 3339 instant (a `datetime` field).
    Time,
}

/// The right side of an `exists` condition.
#[derive(Debug, Clone, PartialEq)]
pub enum ExistsValue {
    /// A value known when the filter is built (never null).
    Value(Value),
    /// A text column of the scanned row.
    Column(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowFilter {
    /// Every row.
    All,
    /// No row.
    Nothing,
    And(Vec<RowFilter>),
    Or(Vec<RowFilter>),
    Not(Box<RowFilter>),
    /// `column <op> value`; false when the column is null. `value` is never
    /// null.
    Compare {
        column: String,
        op: CompareOp,
        value: Value,
        compare_as: CompareAs,
    },
    /// The column is null.
    IsNull(String),
    /// A row of `entity` has every `(column, value)`: text equality, false
    /// when a scanned-row column is null.
    Exists {
        entity: String,
        conditions: Vec<(String, ExistsValue)>,
    },
}

impl RowFilter {
    /// `a && b`, folding constants.
    pub fn and(a: RowFilter, b: RowFilter) -> RowFilter {
        match (a, b) {
            (RowFilter::Nothing, _) | (_, RowFilter::Nothing) => RowFilter::Nothing,
            (RowFilter::All, other) | (other, RowFilter::All) => other,
            (RowFilter::And(mut left), RowFilter::And(right)) => {
                left.extend(right);
                RowFilter::And(left)
            }
            (RowFilter::And(mut left), other) | (other, RowFilter::And(mut left)) => {
                left.push(other);
                RowFilter::And(left)
            }
            (a, b) => RowFilter::And(vec![a, b]),
        }
    }

    /// `a || b`, folding constants.
    pub fn or(a: RowFilter, b: RowFilter) -> RowFilter {
        match (a, b) {
            (RowFilter::All, _) | (_, RowFilter::All) => RowFilter::All,
            (RowFilter::Nothing, other) | (other, RowFilter::Nothing) => other,
            (RowFilter::Or(mut left), RowFilter::Or(right)) => {
                left.extend(right);
                RowFilter::Or(left)
            }
            (RowFilter::Or(mut left), other) | (other, RowFilter::Or(mut left)) => {
                left.push(other);
                RowFilter::Or(left)
            }
            (a, b) => RowFilter::Or(vec![a, b]),
        }
    }

    /// `!a`, folding constants.
    pub fn negate(a: RowFilter) -> RowFilter {
        match a {
            RowFilter::All => RowFilter::Nothing,
            RowFilter::Nothing => RowFilter::All,
            RowFilter::Not(inner) => *inner,
            other => RowFilter::Not(Box::new(other)),
        }
    }

    /// A SQL condition over the scanned row (aliased `t`) that keeps every
    /// row this filter keeps. A part `dialect` cannot write keeps every row
    /// (no row under an odd number of `NOT`s), so the condition stays a
    /// superset, and [`RenderedFilter::exact`] is false.
    pub fn to_sql(&self, dialect: &mut dyn SqlDialect) -> RenderedFilter {
        let mut exact = true;
        let sql = self.write_sql(dialect, false, &mut exact);
        RenderedFilter { sql, exact }
    }

    fn write_sql(&self, dialect: &mut dyn SqlDialect, negated: bool, exact: &mut bool) -> String {
        let written = match self {
            RowFilter::All => Some("TRUE".to_string()),
            RowFilter::Nothing => Some("FALSE".to_string()),
            RowFilter::And(parts) | RowFilter::Or(parts) => {
                let joiner = if matches!(self, RowFilter::And(_)) {
                    " AND "
                } else {
                    " OR "
                };
                let parts: Vec<String> = parts
                    .iter()
                    .map(|part| part.write_sql(dialect, negated, exact))
                    .collect();
                Some(format!("({})", parts.join(joiner)))
            }
            RowFilter::Not(inner) => Some(format!(
                "(NOT {})",
                inner.write_sql(dialect, !negated, exact)
            )),
            RowFilter::IsNull(column) => dialect.is_null(column),
            RowFilter::Compare {
                column,
                op,
                value,
                compare_as,
            } => dialect.compare(column, *op, value, *compare_as, negated),
            RowFilter::Exists { entity, conditions } => dialect.exists(entity, conditions),
        };
        written.unwrap_or_else(|| {
            *exact = false;
            // Keep more rows: TRUE, which a `NOT` above turns into FALSE.
            if negated { "FALSE" } else { "TRUE" }.to_string()
        })
    }
}

/// SQL from [`RowFilter::to_sql`].
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedFilter {
    pub sql: String,
    /// Every part was written as asked.
    pub exact: bool,
}

/// How one store writes the parts of a [`RowFilter`]. Each method returns
/// a condition that holds exactly when the part holds for the scanned row
/// `t` (as a row check reads the stored row), or `None` when the store
/// cannot write it.
pub trait SqlDialect {
    /// [`RowFilter::Compare`]: false when the column reads as null.
    /// `negated` is true under an odd number of `NOT`s: the condition must
    /// then be false, not SQL `NULL`, where the column is null.
    fn compare(
        &mut self,
        column: &str,
        op: CompareOp,
        value: &Value,
        compare_as: CompareAs,
        negated: bool,
    ) -> Option<String>;

    /// [`RowFilter::IsNull`]: the column reads as null.
    fn is_null(&mut self, column: &str) -> Option<String>;

    /// [`RowFilter::Exists`].
    fn exists(&mut self, entity: &str, conditions: &[(String, ExistsValue)]) -> Option<String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes every compare as its column and refuses the column `skip`.
    struct Plain;
    impl SqlDialect for Plain {
        fn compare(
            &mut self,
            column: &str,
            _op: CompareOp,
            _value: &Value,
            _compare_as: CompareAs,
            negated: bool,
        ) -> Option<String> {
            (column != "skip").then(|| format!("{column}{}", if negated { "!" } else { "" }))
        }
        fn is_null(&mut self, column: &str) -> Option<String> {
            (column != "skip").then(|| format!("{column} IS NULL"))
        }
        fn exists(&mut self, entity: &str, _: &[(String, ExistsValue)]) -> Option<String> {
            Some(format!("EXISTS {entity}"))
        }
    }

    fn eq(column: &str) -> RowFilter {
        RowFilter::Compare {
            column: column.into(),
            op: CompareOp::Eq,
            value: Value::String("x".into()),
            compare_as: CompareAs::Text,
        }
    }

    #[test]
    fn a_part_the_store_cannot_write_keeps_more_rows() {
        let filter = RowFilter::and(
            eq("a"),
            RowFilter::negate(RowFilter::or(eq("b"), eq("skip"))),
        );
        let rendered = filter.to_sql(&mut Plain);
        assert_eq!(rendered.sql, "(a AND (NOT (b! OR FALSE)))");
        assert!(!rendered.exact);

        let rendered = RowFilter::or(eq("skip"), RowFilter::IsNull("c".into())).to_sql(&mut Plain);
        assert_eq!(rendered.sql, "(TRUE OR c IS NULL)");

        let exists = RowFilter::Exists {
            entity: "M".into(),
            conditions: vec![],
        };
        let rendered = RowFilter::and(eq("a"), exists).to_sql(&mut Plain);
        assert_eq!(rendered.sql, "(a AND EXISTS M)");
        assert!(rendered.exact);
    }
}
