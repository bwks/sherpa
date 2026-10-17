//! Guest configuration documents that guests read as YAML.
//!
//! cloud-init, cloudbase-init and netplan parse YAML with YAML 1.1 rules, where unquoted
//! strings such as an all-digit MAC address (`52:54:00:21:47:08`, a base-60 integer) or
//! `yes`/`no`/`on`/`off` (booleans) change type. Documents are written as JSON instead:
//! JSON is a subset of YAML in which every string is quoted.

use anyhow::Result;
use serde::Serialize;

/// Render a document guests parse as YAML.
pub(crate) fn to_guest_yaml<T: Serialize>(document: &T) -> Result<String> {
    let mut output = serde_json::to_string_pretty(document)?;
    output.push('\n');
    Ok(output)
}

/// Render cloud-config user-data; cloud-init requires the `#cloud-config` first line.
pub(crate) fn to_cloud_config<T: Serialize>(document: &T) -> Result<String> {
    Ok(format!("#cloud-config\n{}", to_guest_yaml(document)?))
}
