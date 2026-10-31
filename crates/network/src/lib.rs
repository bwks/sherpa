#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

pub(crate) mod linux;

pub mod ebpf;
pub mod tap;
pub mod tc;

pub use linux::{
    create_bridge, create_veth_pair, delete_interface, enslave_to_bridge, find_interfaces_fuzzy,
    set_link_down,
};

pub use ebpf::attach_p2p_redirect;
pub use tap::{create_tap, create_veth_in_netns, get_ifindex};
pub use tc::{apply_link_netem, apply_netem, remove_netem, update_netem};
