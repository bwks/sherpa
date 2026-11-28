//! Sandboxes: the container models that need root-like privileges each run on the Docker of a
//! Flatcar VM of their own, which Ignition and a config drive configure
//! (`docs/architecture/SANDBOX.md`).

mod base;
mod create;
mod guest;
mod status;
mod store;
mod sweep;

pub(crate) use base::{SandboxBase, sandbox_base};
pub(crate) use create::{
    SANDBOX_MEMORY_OVERHEAD_MIB, SandboxCreate, SandboxHost, create_sandbox, sandbox_tuning,
};
pub(crate) use guest::DRIVE_VOLUME_DEPTH;
pub(crate) use status::StatusWatch;
pub(crate) use sweep::remove_unfinished_builds;
