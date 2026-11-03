//! Bridge tables schema
//!
//! - A bridge's index is unique within its lab; deleting the lab deletes the bridge.
//! - `bridge_member` holds a bridge's members in order. Deleting the bridge, or a member's
//!   node, deletes the membership.
pub(crate) const TABLES: &str = include_str!("../../sql/schema/bridge.sql");
