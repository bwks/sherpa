//! Link table schema
//!
//! - A link joins `int_a` on `node_a` to `int_b` on `node_b`; that combination is unique.
//! - Deleting either node, or the lab, deletes the link.
//! - Impairments are stored per direction, `a_to_b_*` and `b_to_a_*`.

pub(crate) const TABLES: &str = include_str!("../../sql/schema/link.sql");
