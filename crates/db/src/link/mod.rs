//! Link CRUD operations
//!
//! This module provides create, read, update, and delete operations for links.
//! Links represent network connections between two nodes in a lab.

mod create;
mod read;
mod update;

pub use create::create_link;
pub use read::list_links_by_lab;
pub use update::update_link_impairments;
