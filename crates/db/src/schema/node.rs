//! Node table schema
//!
//! - Node names and indexes are unique within a lab, and VNC ports across all labs.
//! - `image` refers to the node image, which cannot be deleted while a node uses it.
//! - Deleting a lab deletes its nodes; deleting a node deletes its links and bridge memberships.
use anyhow::Result;
use shared::data::NodeState;

use super::helpers::{enum_rule, rule_triggers};

const TABLE: &str = "node";

pub(crate) const TABLES: &str = include_str!("../../sql/schema/node.sql");

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    let rules = [enum_rule(TABLE, "state", NodeState::to_vec(), false)?];
    Ok(rule_triggers(TABLE, &rules))
}
