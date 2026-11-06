//! Bridge CRUD operations
//!
//! This module provides create, read, update, and delete operations
//! for shared bridge records.

mod create;
mod read;

pub use create::create_bridge;
pub use read::list_bridges;
