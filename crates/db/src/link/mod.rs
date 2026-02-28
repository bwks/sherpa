//! Link CRUD operations
//!
//! This module provides create, read, update, and delete operations for links.
//! Links represent network connections between two nodes in a lab.

mod create;
mod delete;
mod read;
mod update;

pub use create::create_link;
pub use delete::{delete_link, delete_link_by_id, delete_links_by_lab, delete_links_by_node};
pub use read::{
    count_links, count_links_by_lab, count_links_by_node, get_link, get_link_by_id,
    get_link_by_peers, list_links, list_links_by_lab, list_links_by_node,
};
pub use update::update_link;
