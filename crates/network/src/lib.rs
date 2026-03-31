#![deny(clippy::unwrap_used, clippy::expect_used)]

mod linux;

pub use linux::{
    create_bridge, create_veth_pair, delete_interface, enslave_to_bridge, find_interfaces_fuzzy,
};
