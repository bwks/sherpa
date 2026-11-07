//! Node table schema
//!
//! - Node names and indexes are unique within a lab, and VNC ports across all labs.
//! - `image` refers to the node image, which cannot be deleted while a node uses it.
//! - Deleting a lab deletes its nodes; deleting a node deletes its links and bridge memberships.

pub(crate) const TABLES: &str = include_str!("../../sql/schema/node.sql");
