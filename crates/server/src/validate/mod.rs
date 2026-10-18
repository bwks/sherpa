//! Validators that only the server enforces.
//!
//! Validators shared with other crates live in `shared::validators`, and manifest checks
//! shared with the client live in `topology::validate`.

mod environment;
mod node_image;
mod tailscale_gateway;
mod version;

pub use environment::validate_environment_variables;
pub use node_image::validate_node_image_update;
pub use tailscale_gateway::validate_tailscale_gateway_settings;
pub use version::validate_and_resolve_node_versions;
