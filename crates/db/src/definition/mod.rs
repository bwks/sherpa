//! A lab's definition: every parameter of the manifest it was deployed from, as the user wrote
//! it, stored in the lab's and nodes' rows and the `manifest_*` tables, and read back as that
//! manifest. File contents and resolved variable values are never stored.

mod read;
mod write;

pub use read::read_lab_definition;
pub use write::{store_lab_definition, store_redeployed_node};

#[cfg(test)]
mod tests;
