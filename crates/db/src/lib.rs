pub mod bridge;
mod connect;
mod helpers;
pub mod lab;
pub mod link;
pub mod node;
pub mod node_image;
pub mod schema;
pub mod seed;
pub mod user;

pub use connect::connect;
pub use shared::data::{DbBridge, DbLab, DbLink, DbNode, DbUser, NodeConfig};

// Helper functions for extracting IDs safely
pub use helpers::{get_image_id, get_lab_id, get_node_id, get_user_id};

// Lab CRUD operations
pub use lab::{
    count_labs, count_labs_by_user, create_lab, delete_lab, delete_lab_by_id, delete_lab_cascade,
    delete_lab_links, delete_lab_nodes, delete_lab_safe, get_lab, get_lab_by_id,
    get_lab_by_name_and_user, get_lab_owner_username, get_used_loopback_networks,
    get_used_management_networks, list_labs, list_labs_by_user, update_lab, upsert_lab,
    validate_lab_id,
};

// Node image CRUD operations
pub use node_image::{
    count_node_images, create_node_image, delete_node_image, get_default_node_image,
    get_node_image_by_id, get_node_image_by_model_kind_version, get_node_image_versions,
    list_node_images, list_node_images_by_kind, update_node_image, upsert_node_image,
};

// Node CRUD operations
pub use node::{
    count_nodes, count_nodes_by_lab, create_node, delete_node, delete_node_by_id,
    delete_node_cascade, delete_node_links, delete_node_safe, delete_nodes_by_lab, get_node,
    get_node_by_id, get_node_by_name_and_lab, list_nodes, list_nodes_by_lab, update_node,
    update_node_mgmt_ipv4, update_node_state,
};

// Link CRUD operations
pub use link::{
    count_links, count_links_by_lab, count_links_by_node, create_link, delete_link,
    delete_link_by_id, delete_links_by_lab, delete_links_by_node, get_link, get_link_by_id,
    get_link_by_peers, list_links, list_links_by_lab, list_links_by_node, update_link,
};

pub use schema::apply_schema;
pub use seed::admin_user::seed_admin_user;

// User CRUD operations
pub use user::{
    count_users, create_user, delete_user, delete_user_by_username, delete_user_safe, get_user,
    get_user_by_id, get_user_for_auth, list_users, update_user, upsert_user,
};

// Bridge CRUD operations
pub use bridge::{
    create_bridge, delete_bridge, delete_lab_bridges, get_bridge, get_bridge_by_index, list_bridges,
};
