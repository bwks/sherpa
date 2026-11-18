#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

pub mod bridge;
// The placeholder rules `build.rs` applies, built here too so `cargo test -p db` runs their tests.
#[cfg(test)]
#[path = "../build/placeholders.rs"]
mod build_placeholders;
mod connect;
pub mod cpu_pin;
pub mod definition;
mod helpers;
pub mod lab;
pub mod link;
pub mod node;
pub mod node_image;
mod persistence;
mod queries;
pub mod schema;
pub mod secret;
pub mod seed;
pub mod session;
mod sql;
pub mod talos;
pub mod user;

pub use connect::{Database, open, open_in_memory};
pub use cpu_pin::{CpuPinTaken, claim_cpu_pins, list_cpu_pins, retain_cpu_pins};
pub use definition::{read_lab_definition, store_lab_definition, store_redeployed_node};
pub use secret::{SecretKey, secret_key_registered, verify_secret_key};
pub use shared::data::{
    DbBridge, DbCpuPins, DbLab, DbLink, DbNode, DbSession, DbSessionUser, DbUser, NodeConfig,
    SessionChannel,
};

// Helper functions for extracting IDs safely
pub use helpers::{get_image_id, get_lab_id, get_node_id, get_user_id};

// Lab CRUD operations
pub use lab::{
    LabDetails, LabNetworks, count_labs_by_user, create_lab, delete_lab_from, find_lab, get_lab,
    get_used_ipv6_loopback_networks, get_used_ipv6_management_networks,
    get_used_management_networks, list_labs, list_labs_by_user, update_lab_phase_from,
    update_lab_tailscale_container, validate_lab_id,
};

// Node image CRUD operations
pub use node_image::{
    DefaultRequest, LastDefaultImage, count_node_images, delete_node_image,
    find_default_node_image, find_node_image_by_id, find_node_image_by_model_kind_version,
    get_node_image_versions, list_node_images, list_node_images_by_ids, list_node_images_by_kind,
    update_node_image, upsert_node_image,
};

// Node CRUD operations
pub use node::{
    count_nodes_by_lab, count_nodes_by_ssh_username, create_node, get_node,
    get_node_by_name_and_lab, list_node_states_by_user, list_nodes_by_lab, update_node_host_key,
    update_node_image_ref, update_node_mgmt_ipv4, update_node_mgmt_ipv6, update_node_mgmt_mac,
    update_node_ready_check, update_node_resources, update_node_ssh_username,
    update_node_state_from,
};

// Link CRUD operations
pub use link::{create_link, list_links_by_lab, update_link_impairments};

pub use schema::{SCHEMA_VERSION, apply_schema, check_schema_upgrade};
pub use seed::admin_user::seed_admin_user;

// User CRUD operations
pub use user::{
    DevicePassword, UserChanged, change_user_password, create_admin_user, create_user, delete_user,
    device_password_set, find_user, find_user_by_id, get_user, list_users, open_device_password,
    set_device_password, update_user,
};

// Session operations
pub use session::{create_session, delete_expired_sessions, delete_session, find_session};

// Bridge CRUD operations
pub use bridge::{create_bridge, list_bridges};

// Talos cluster plans
pub use talos::{list_talos_cluster_plans, store_talos_cluster_plans};
