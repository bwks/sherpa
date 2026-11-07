//! Node image table schema
//!
//! One row per imported image version of a node model. `(model, kind, version)` is unique.
//! A node image used by a node cannot be deleted (`node.image` is `ON DELETE RESTRICT`).

pub(crate) const TABLES: &str = include_str!("../../sql/schema/node_image.sql");
