//! Manifest validation shared by the client and the server.
//!
//! These checks take manifest types, so they live with the manifest rather than in
//! `shared::validators`. Validators only inspect the data they are given; they do not
//! generate data.

mod device;
mod ipv6;
mod link;
mod tailscale;
mod talos_cluster;

pub use device::{check_device_count, check_duplicate_device};
pub use ipv6::validate_manifest_ipv6_addresses;
pub use link::{
    check_bridge_device, check_duplicate_interface_link, check_interface_bounds, check_link_device,
    check_mgmt_usage,
};
pub use tailscale::validate_tailscale;
pub use talos_cluster::validate_talos_clusters;
