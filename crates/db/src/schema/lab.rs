//! Lab table schema
//!
//! - `lab_id` is unique, as are `(name, user)` and each of the lab's networks.
//! - `user` refers to the owner, who cannot be deleted while the lab exists.
//! - Deleting a lab deletes its nodes, links and bridges (their `lab` columns cascade).
use anyhow::Result;
use shared::data::LabState;

use super::helpers::{enum_rule, rule_triggers};

const TABLE: &str = "lab";

pub(crate) const TABLES: &str = include_str!("../../sql/schema/lab.sql");

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    let rules = [enum_rule(TABLE, "status", LabState::to_vec(), false)?];
    Ok(rule_triggers(TABLE, &rules))
}
