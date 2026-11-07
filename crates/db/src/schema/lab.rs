//! Lab table schema
//!
//! - `lab_id` is unique, as are `(name, user)` and each of the lab's networks.
//! - `user` refers to the owner, who cannot be deleted while the lab exists.
//! - Deleting a lab deletes its nodes, links and bridges (their `lab` columns cascade).

pub(crate) const TABLES: &str = include_str!("../../sql/schema/lab.sql");
