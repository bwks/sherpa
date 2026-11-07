//! Lab creation: `up`.

mod context;
mod lab;
mod links;
mod networks;
mod nodes;
mod plan;
mod readiness;
mod records;
mod runtime;
mod services;
mod ssh;
mod ztp;

pub use lab::up_lab;
pub(crate) use nodes::process_manifest_nodes;
