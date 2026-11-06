//! Manifest validation shared by the client and the server.
//!
//! These checks take manifest types, so they live with the manifest rather than in
//! `shared::validators`. Validators only inspect the data they are given; they do not
//! generate data.

mod device;
mod link;
mod performance_tuning;
mod provisioner;
mod ptx_platform;
mod tailscale;
mod talos_cluster;
mod text_files;
mod watchdog;

pub use device::{check_device_count, check_duplicate_device};
pub use link::{
    check_bridge_device, check_duplicate_interface_link, check_interface_bounds, check_link_device,
    check_link_impairments, check_mgmt_usage, check_no_p2p_field,
};
pub use performance_tuning::{check_cpu_pin_overlap, check_performance_tuning};
pub use provisioner::validate_provisioners;
pub use ptx_platform::check_ptx_platform;
pub use tailscale::validate_tailscale;
pub use talos_cluster::validate_talos_clusters;
pub use text_files::check_text_file_modes;
pub use watchdog::check_watchdog;
