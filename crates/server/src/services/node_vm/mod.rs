//! Node VMs: the container models that need root-like privileges each run in a Flatcar VM of
//! their own, on that VM's Docker, so the privileges reach the VM's kernel and not the server's.
//! sherpad configures the VM with Ignition and a config drive, and a unit in the VM starts the
//! container and reports how that went. See `docs/architecture/NODE_VM.md`.

mod base;
mod create;
mod guest;
mod status;
mod store;

pub(crate) use base::require_base_image;
pub(crate) use create::{NODE_VM_MEMORY_OVERHEAD_MIB, NodeVmCreate, NodeVmHost, create_node_vm};
pub(crate) use status::StatusWatch;
pub(crate) use store::remove_unfinished_store_builds;
