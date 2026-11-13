#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

pub(crate) mod linux;

pub mod ebpf;
pub mod ethtool;
pub mod tap;
pub mod tc;

pub use linux::{
    container_veth_peer_ifindex, create_bridge, create_veth_pair, delete_interface,
    enslave_to_bridge, list_interface_names, set_link_down,
};

pub use ebpf::{attach_p2p_redirect, has_p2p_redirect};
pub use ethtool::disable_tx_checksum_offload;
pub use tap::{NetnsVethState, create_tap, create_veth_in_netns, find_ifindex, get_ifindex};
pub use tc::{NetemRemoval, apply_netem, remove_netem, sync_netem, update_netem};
