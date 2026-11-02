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
pub(crate) fn sql_values<T: Serialize>(values: Vec<T>, field: &str) -> Result<String> {
    let names = values
        .iter()
        .map(|value| enum_text(value, field).map(|name| sql_text(&name)))
        .collect::<Result<Vec<_>>>()?;
    Ok(names.join(", "))
}

/// A trigger statement refusing a row whose `column` is not one of `values`.
pub(crate) fn enum_rule<T: Serialize>(
    table: &str,
    column: &str,
    values: Vec<T>,
    nullable: bool,
) -> Result<String> {
    let allowed = sql_values(values, column)?;
    let present = if nullable {
        format!("NEW.{column} IS NOT NULL AND ")
    } else {
        String::new()
    };
    Ok(format!(
        "SELECT RAISE(ABORT, {}) WHERE {present}NEW.{column} NOT IN ({allowed});",
        sql_text(&format!("Invalid value for {table}.{column}"))
    ))
}

/// Drop and create the insert and update triggers that apply `rules` to `table`.
pub(crate) fn rule_triggers(table: &str, rules: &[String]) -> String {
    let body = rules.join("\n    ");
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
    fn enum_rule_lists_stored_names() {
        let rule = enum_rule("node_image", "kind", NodeKind::to_vec(), false).unwrap();
        assert!(rule.contains("NEW.kind NOT IN ("));
        assert!(rule.contains(&sql_text(&enum_text(&NodeKind::Container, "kind").unwrap())));
        assert!(!rule.contains("IS NOT NULL"));
        let nullable = enum_rule("node_image", "kind", NodeKind::to_vec(), true).unwrap();
        assert!(nullable.contains("NEW.kind IS NOT NULL AND NEW.kind NOT IN ("));
    }
}
