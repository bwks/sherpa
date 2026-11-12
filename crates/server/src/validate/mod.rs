//! Validators that only the server enforces.
//!
//! Validators shared with other crates live in `shared::validators`, and manifest checks
//! shared with the client live in `topology::validate`.

mod environment;
mod image_scan;
mod lab_origin;
mod memory;
mod node_image;
mod node_secrets;
mod performance_tuning;
mod provisioner;
mod tailscale_gateway;
mod version;
mod ztp;

pub use environment::validate_environment_variables;
pub use image_scan::validate_image_scan;
pub use lab_origin::validate_lab_origin;
pub use memory::{CommittedMemory, MemoryClaim, check_host_memory_settings, check_memory_fits};
pub use node_image::{validate_image_version, validate_node_image_update};
pub use node_secrets::{validate_node_secrets, validate_secret_files};
pub use performance_tuning::check_host_resources;
pub use provisioner::reject_provisioners;
pub use tailscale_gateway::validate_tailscale_gateway_settings;
pub use version::{validate_and_resolve_node_versions, validate_container_repo};
pub use ztp::validate_node_ztp;
