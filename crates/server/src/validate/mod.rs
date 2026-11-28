//! Validators that only the server enforces.
//!
//! Validators shared with other crates live in `shared::validators`, and manifest checks
//! shared with the client live in `topology::validate`.

mod device_password;
mod host_resources;
mod image_scan;
mod lab_origin;
mod login_name;
mod management_prefix;
mod memory;
mod node_image;
mod node_secrets;
mod provisioner;
mod sandbox_volume;
mod service_account;
mod tailscale_gateway;
mod version;
mod web_redeploy;
mod ztp;

pub use device_password::{DevicePasswordRule, validate_device_password};
pub use host_resources::{check_container_cpus, check_host_resources};
pub use image_scan::validate_image_scan;
pub use lab_origin::validate_lab_origin;
pub use login_name::validate_login_name;
pub use management_prefix::{validate_management_prefix_ipv4, validate_management_prefix_ipv6};
pub use memory::{
    CommittedMemory, MemoryClaim, MemoryShortage, check_host_memory_settings, check_memory_fits,
};
pub use node_image::{validate_image_version, validate_node_image_update};
pub use node_secrets::{validate_node_secrets, validate_secret_files};
pub use provisioner::reject_provisioners;
pub use sandbox_volume::{sandbox_volume_sources, validate_sandbox_volumes};
pub use service_account::validate_service_account;
pub use tailscale_gateway::validate_tailscale_gateway_settings;
pub use version::{
    validate_and_resolve_node_versions, validate_container_repo, validate_sandbox_image,
};
pub use web_redeploy::check_web_redeployable;
pub use ztp::validate_node_ztp;
