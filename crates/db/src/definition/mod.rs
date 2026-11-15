//! A lab's definition: every parameter of the manifest it was deployed from, stored in the
//! lab's and nodes' rows and the `manifest_*` tables, and read back as that manifest.

mod read;
mod write;

pub use read::{list_labs_without_definition, read_lab_definition};
pub use write::{StoredProvisioning, store_lab_definition, store_redeployed_node};

#[cfg(test)]
mod tests;
