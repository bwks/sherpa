//! P2p link attachment: the state machine of a link whose end was made again, the one
//! function that re-attaches a node's links after any event that makes its ends again, and the
//! host operations it uses. See `docs/architecture/STATE_MACHINES.md`, section 25.

#[cfg(test)]
pub(crate) mod fake;
mod host;
mod reattach;
mod transition;
mod veths;

pub(crate) use host::{KernelHost, LinkHost};
pub(crate) use reattach::{
    LabLinks, LinkWiring, RecreatedEnd, attach_link, reattach_node_links, report_links,
};
pub(crate) use veths::{
    ContainerNode, ContainerVeth, container_veths, data_interface_indices, node_bridge_interfaces,
    node_links,
};
