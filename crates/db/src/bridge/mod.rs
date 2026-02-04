//! Bridge CRUD operations
//!
//! This module provides create, read, update, and delete operations
//! for shared bridge records.

mod create;
mod delete;
mod read;

pub use create::create_bridge;
pub use delete::{delete_bridge, delete_lab_bridges};
pub use read::{get_bridge, get_bridge_by_index, list_bridges};
