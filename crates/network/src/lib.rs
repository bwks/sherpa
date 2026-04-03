#![deny(clippy::unwrap_used, clippy::expect_used)]
#![forbid(unsafe_code)]

pub(crate) mod linux;

pub mod ebpf;
pub mod tap;
pub mod tc;

pub use linux::{
    create_bridge, create_veth_pair, delete_interface, enslave_to_bridge, find_interfaces_fuzzy,
    set_link_down,
};

pub use ebpf::attach_p2p_redirect;
pub use tap::{create_tap, get_ifindex, move_to_netns};
pub use tc::{LinkImpairment, apply_netem, remove_netem, update_netem};
