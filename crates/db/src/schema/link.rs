//! Link table schema
//!
//! - A link joins `int_a` on `node_a` to `int_b` on `node_b`; that combination is unique.
//! - Deleting either node, or the lab, deletes the link.
//! - Impairments are stored per direction, `a_to_b_*` and `b_to_a_*`.
use anyhow::Result;
use shared::data::BridgeKind;

use super::helpers::{EnumColumn, enum_column, rule_triggers};

const TABLE: &str = "link";

pub(crate) const TABLES: &str = include_str!("../../sql/schema/link.sql");

/// The enum columns, and the values the current Rust enums allow in them.
pub(crate) fn columns() -> Result<Vec<EnumColumn>> {
    Ok(vec![enum_column(
        TABLE,
        "kind",
        &BridgeKind::to_vec(),
        false,
    )?])
}

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    Ok(rule_triggers(TABLE, &columns()?))
}
