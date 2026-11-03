//! Link table schema
//!
//! - A link joins `int_a` on `node_a` to `int_b` on `node_b`; that combination is unique.
//! - Deleting either node, or the lab, deletes the link.
//! - Impairments are stored per direction, `a_to_b_*` and `b_to_a_*`.
use anyhow::Result;
use shared::data::BridgeKind;

use super::helpers::{enum_rule, rule_triggers};

const TABLE: &str = "link";

pub(crate) const TABLES: &str = include_str!("../../sql/schema/link.sql");

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    let rules = [enum_rule(TABLE, "kind", BridgeKind::to_vec(), false)?];
    Ok(rule_triggers(TABLE, &rules))
}
