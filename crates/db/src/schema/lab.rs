//! Lab table schema
//!
//! - `lab_id` is unique, as are `(name, user)` and each of the lab's networks.
//! - `user` refers to the owner, who cannot be deleted while the lab exists.
//! - Deleting a lab deletes its nodes, links and bridges (their `lab` columns cascade).
use anyhow::Result;
use shared::data::LabState;

use super::helpers::{EnumColumn, enum_column, rule_triggers};

const TABLE: &str = "lab";

pub(crate) const TABLES: &str = include_str!("../../sql/schema/lab.sql");

/// The enum columns, and the values the current Rust enums allow in them.
pub(crate) fn columns() -> Result<Vec<EnumColumn>> {
    Ok(vec![enum_column(
        TABLE,
        "status",
        &LabState::to_vec(),
        false,
    )?])
}

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    Ok(rule_triggers(TABLE, &columns()?))
}
