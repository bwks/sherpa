//! A lab's definition: every parameter of the manifest it was deployed from, as the user wrote
//! it, stored in the lab's and nodes' rows and the `manifest_*` tables, and read back as that
//! manifest. File contents and resolved variable values are never stored.

mod imported;
mod read;
mod write;

pub use imported::{mark_definition_imported, node_definition_imported};
pub use read::{list_labs_without_definition, read_lab_definition};
pub use write::{ImportMark, store_lab_definition, store_redeployed_node};

#[cfg(test)]
mod tests;
