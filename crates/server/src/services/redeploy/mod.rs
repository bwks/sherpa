//! Redeploying one node of a running lab: the node is destroyed and built again with a fresh
//! configuration, keeping its addresses and links.

mod container;
mod context;
mod links;
mod node;
mod teardown;
mod vm;

pub use node::redeploy_node;
