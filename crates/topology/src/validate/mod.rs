//! Manifest validation shared by the client and the server.
//!
//! These checks take manifest types, so they live with the manifest rather than in
//! `shared::validators`. Validators only inspect the data they are given; they do not
//! generate data.
//!
//! The client runs them before it sends a manifest, so mistakes are reported without a round
//! trip. The server runs them again and is the authority: a client can be old, modified or
//! bypassed by calling the API.

mod client_files;
mod description;
mod device;
mod link;
mod names;
mod node_kind;
mod nonzero;
mod provisioner;
mod ptx_platform;
mod tailscale;
mod talos_cluster;
mod text_files;
mod vm_tuning;
mod ztp;

pub use client_files::check_client_read_files;
pub use description::check_lab_description;
pub use device::{check_device_count, check_duplicate_device};
pub use link::{
    check_duplicate_interface_link, check_interface_bounds, check_link_impairments,
    check_no_p2p_field,
};
pub use names::{check_lab_name, check_node_names};
pub use node_kind::{check_node_kind_settings, check_node_model_kind};
pub use nonzero::{check_nonzero_node_settings, check_ready_timeout};
pub use provisioner::validate_provisioners;
pub use ptx_platform::check_ptx_platform;
pub use tailscale::validate_tailscale;
pub use talos_cluster::validate_talos_clusters;
pub use text_files::check_text_file_modes;
pub use vm_tuning::{check_cpu_pin_overlap, check_vm_tuning};
pub use ztp::check_node_ztp;
