//! Sandboxes: the container models that need root-like privileges each run in a Flatcar VM of
//! their own, on that VM's Docker, so the privileges reach the VM's kernel and not the server's.
//! sherpad configures the VM with Ignition and a config drive, and a unit in the VM starts the
//! container and reports how that went. Each VM boots an overlay of a boot disk made once per
//! base image version. See `docs/architecture/SANDBOX.md`.

mod base;
mod boot;
mod create;
mod guest;
mod status;
mod store;
mod sweep;

pub(crate) use base::{SandboxBase, sandbox_base};
pub(crate) use boot::BootDisk;
pub(crate) use create::{SANDBOX_MEMORY_OVERHEAD_MIB, SandboxCreate, SandboxHost, create_sandbox};
pub(crate) use status::StatusWatch;
pub(crate) use sweep::remove_unfinished_builds;
