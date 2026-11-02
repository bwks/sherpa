//! Link table schema
//!
//! - A link joins `int_a` on `node_a` to `int_b` on `node_b`; that combination is unique.
//! - Deleting either node, or the lab, deletes the link.
//! - Impairments are stored per direction, `a_to_b_*` and `b_to_a_*`.
use anyhow::Result;
use shared::data::BridgeKind;

use super::helpers::{enum_rule, rule_triggers};

const TABLE: &str = "link";

pub(crate) const TABLES: &str = "
CREATE TABLE link (
    id TEXT PRIMARY KEY,
    \"index\" INTEGER NOT NULL CHECK (\"index\" BETWEEN 0 AND 65535),
    kind TEXT NOT NULL,
    node_a TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    node_b TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    int_a TEXT NOT NULL,
    int_b TEXT NOT NULL,
    lab TEXT NOT NULL REFERENCES lab(id) ON DELETE CASCADE,
    tap_a TEXT NOT NULL,
    tap_b TEXT NOT NULL,
    a_to_b_delay_us INTEGER NOT NULL CHECK (a_to_b_delay_us >= 0),
    a_to_b_jitter_us INTEGER NOT NULL CHECK (a_to_b_jitter_us >= 0),
    a_to_b_loss_percent REAL NOT NULL,
    a_to_b_reorder_percent REAL NOT NULL,
    a_to_b_corrupt_percent REAL NOT NULL,
    b_to_a_delay_us INTEGER NOT NULL CHECK (b_to_a_delay_us >= 0),
    b_to_a_jitter_us INTEGER NOT NULL CHECK (b_to_a_jitter_us >= 0),
    b_to_a_loss_percent REAL NOT NULL,
    b_to_a_reorder_percent REAL NOT NULL,
    b_to_a_corrupt_percent REAL NOT NULL,
    UNIQUE (node_a, node_b, int_a, int_b)
) STRICT;
CREATE INDEX link_node_b ON link (node_b);
CREATE INDEX link_lab ON link (lab);
";

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    let rules = [enum_rule(TABLE, "kind", BridgeKind::to_vec(), false)?];
    Ok(rule_triggers(TABLE, &rules))
}
