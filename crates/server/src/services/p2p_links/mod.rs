//! P2p link attachment: the state machine of a link whose end was made again, the one
//! function that re-attaches a node's links after any event that makes its ends again, the
//! scanner's check and repair of ends made again outside Sherpa, the carrier of each end
//! following its link's state, and the host operations they use. See
//! `docs/architecture/STATE_MACHINES.md`, section 25.

mod carrier;
#[cfg(test)]
pub(crate) mod fake;
mod host;
mod plan;
mod reattach;
mod repair;
mod transition;
mod veths;

pub(crate) use carrier::follow_carrier;
pub(crate) use host::{KernelHost, LinkHost};
pub(crate) use plan::{PlannedNode, load_lab_manifest, recreated_end};
pub(crate) use reattach::{
    LabLinks, LinkWiring, RecreatedEnd, attach_link, reattach_node_links, report_links,
};
pub(crate) use repair::{check_lab_links, repair_node};
pub(crate) use veths::{
    ContainerNode, ContainerVeth, container_veths, data_interface_indices, node_bridge_interfaces,
    node_links,
};
