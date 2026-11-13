//! Manifest validation shared by the client and the server.
//!
//! These checks take manifest types, so they live with the manifest rather than in
//! `shared::validators`. Validators only inspect the data they are given; they do not
//! generate data.

mod client_files;
mod description;
mod device;
mod link;
mod node_kind;
mod provisioner;
mod ptx_platform;
mod tailscale;
mod talos_cluster;
mod text_files;
mod vm_tuning;
mod ztp;

pub use client_files::check_client_read_files;
pub use description::check_lab_description;
pub use device::{check_device_count, check_duplicate_device, check_node_names};
pub use link::{
    check_duplicate_interface_link, check_interface_bounds, check_link_impairments,
    check_no_p2p_field,
};
pub use node_kind::check_node_kind_settings;
pub use provisioner::validate_provisioners;
pub use ptx_platform::check_ptx_platform;
pub use tailscale::validate_tailscale;
pub use talos_cluster::validate_talos_clusters;
pub use text_files::check_text_file_modes;
pub use vm_tuning::{check_cpu_pin_overlap, check_vm_tuning};
pub use ztp::check_node_ztp;
