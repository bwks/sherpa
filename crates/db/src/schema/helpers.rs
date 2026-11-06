//! Schema helper utilities
//!
//! Enum columns are checked by triggers, not `CHECK` constraints. SQLite cannot change a
//! `CHECK` constraint without rebuilding its table, and the enums gain variants with new node
//! models, so their triggers are dropped and created again on every start from the current
//! Rust enums.
use anyhow::Result;
use serde::Serialize;

use crate::persistence::enum_text;
use crate::sql::sql_text;

/// A comma-separated list of the quoted, stored names of `values`, for an SQL `IN` list.
///
/// # Errors
///
/// Fails when a value does not serialize to its stored name.
pub(crate) fn sql_values<T: Serialize>(values: &[T], field: &str) -> Result<String> {
    let names = values
        .iter()
        .map(|value| enum_text(value, field).map(|name| sql_text(&name)))
        .collect::<Result<Vec<_>>>()?;
    Ok(names.join(", "))
}

/// An enum column, and the stored names of the values the current Rust enum allows in it.
pub(crate) struct EnumColumn {
    table: &'static str,
    column: &'static str,
    allowed: String,
    nullable: bool,
}

/// The enum column `table.column`, which holds one of `values`, or NULL when `nullable`.
pub(crate) fn enum_column<T: Serialize>(
    table: &'static str,
    column: &'static str,
    values: &[T],
    nullable: bool,
) -> Result<EnumColumn> {
    Ok(EnumColumn {
        table,
        column,
        allowed: sql_values(values, column)?,
        nullable,
    })
}

impl EnumColumn {
    /// `table.column`, as errors name it.
    pub(crate) fn name(&self) -> String {
        format!("{}.{}", self.table, self.column)
    }

    /// A trigger statement refusing a row whose value is not allowed.
    pub(crate) fn rule(&self) -> String {
        let present = if self.nullable {
            format!("NEW.{} IS NOT NULL AND ", self.column)
        } else {
            String::new()
        };
        format!(
            "SELECT RAISE(ABORT, {}) WHERE {present}NEW.{} NOT IN ({});",
            sql_text(&format!("Invalid value for {}", self.name())),
            self.column,
            self.allowed
        )
    }

    /// A query for the stored values that are not allowed, as a newer release can leave.
    pub(crate) fn unknown_values_query(&self) -> String {
        let present = if self.nullable {
            format!("{} IS NOT NULL AND ", self.column)
        } else {
            String::new()
        };
        format!(
            "SELECT DISTINCT {column} FROM {table} WHERE {present}{column} NOT IN ({allowed})",
            column = self.column,
            table = self.table,
            allowed = self.allowed
        )
    }
}

/// Drop and create the insert and update triggers that check `columns` of `table`.
pub(crate) fn rule_triggers(table: &str, columns: &[EnumColumn]) -> String {
    let body = columns
        .iter()
        .map(EnumColumn::rule)
        .collect::<Vec<_>>()
        .join("\n    ");
    format!(
        "DROP TRIGGER IF EXISTS {table}_rules_insert;
CREATE TRIGGER {table}_rules_insert BEFORE INSERT ON {table} BEGIN
    {body}
END;
DROP TRIGGER IF EXISTS {table}_rules_update;
CREATE TRIGGER {table}_rules_update BEFORE UPDATE ON {table} BEGIN
    {body}
END;
"
    )
}

#[cfg(test)]
mod tests {
    use shared::data::NodeKind;

    use super::*;

    #[test]
    fn an_enum_rule_lists_stored_names() {
        let column = enum_column("node_image", "kind", &NodeKind::to_vec(), false).unwrap();
        let rule = column.rule();
        assert!(rule.contains("NEW.kind NOT IN ("));
        assert!(rule.contains(&sql_text(&enum_text(&NodeKind::Container, "kind").unwrap())));
        assert!(!rule.contains("IS NOT NULL"));
        assert!(
            column
                .unknown_values_query()
                .contains("WHERE kind NOT IN (")
        );
        let nullable = enum_column("node_image", "kind", &NodeKind::to_vec(), true).unwrap();
        assert!(
            nullable
                .rule()
                .contains("NEW.kind IS NOT NULL AND NEW.kind NOT IN (")
        );
        assert!(
            nullable
                .unknown_values_query()
                .contains("WHERE kind IS NOT NULL AND kind NOT IN (")
        );
    }
}
