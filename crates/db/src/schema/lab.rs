//! Lab table schema
//!
//! - `lab_id` is unique, as are `(name, user)` and each of the lab's networks.
//! - `user` refers to the owner, who cannot be deleted while the lab exists.
//! - Deleting a lab deletes its nodes, links and bridges (their `lab` columns cascade).
use anyhow::Result;
use shared::data::LabState;

use super::helpers::{enum_rule, rule_triggers};

const TABLE: &str = "lab";

pub(crate) const TABLES: &str = "
CREATE TABLE lab (
    id TEXT PRIMARY KEY,
    lab_id TEXT NOT NULL UNIQUE CHECK (length(lab_id) >= 1),
    name TEXT NOT NULL,
    user TEXT NOT NULL REFERENCES user(id) ON DELETE RESTRICT,
    loopback_network TEXT NOT NULL UNIQUE,
    management_network TEXT NOT NULL UNIQUE,
    gateway_ipv4 TEXT NOT NULL,
    router_ipv4 TEXT NOT NULL,
    tailscale_container_id TEXT,
    management_network_v6 TEXT UNIQUE,
    gateway_ipv6 TEXT,
    router_ipv6 TEXT,
    loopback_network_v6 TEXT UNIQUE,
    status TEXT NOT NULL,
    UNIQUE (name, user)
) STRICT;
CREATE INDEX lab_user ON lab (user);
";

/// The triggers checking the enum columns against the current Rust enums.
pub(crate) fn rules() -> Result<String> {
    let rules = [enum_rule(TABLE, "status", LabState::to_vec(), false)?];
    Ok(rule_triggers(TABLE, &rules))
}
