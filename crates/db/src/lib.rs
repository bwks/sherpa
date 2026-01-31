mod action;
mod connect;
mod helpers;
pub mod lab;
pub mod node;
pub mod node_config;
pub mod schema;
pub mod seed;
pub mod user;

pub use action::{create_lab_link, create_lab_node};
pub use connect::connect;
pub use data::{DbLab, DbLink, DbNode, DbUser, NodeConfig};

// Lab CRUD operations
pub use lab::{
    count_labs, count_labs_by_user, create_lab, delete_lab, delete_lab_by_id, delete_lab_cascade,
    delete_lab_links, delete_lab_nodes, delete_lab_safe, get_lab, get_lab_by_id,
    get_lab_by_name_and_user, list_labs, list_labs_by_user, update_lab, upsert_lab,
    validate_lab_id,
};

// Node config CRUD operations
pub use node_config::{
    count_node_configs, create_node_config, delete_node_config, get_node_config_by_id,
    get_node_config_by_model_kind, list_node_configs, update_node_config, upsert_node_config,
};

// Node CRUD operations
pub use node::{
    count_nodes, count_nodes_by_lab, create_node, delete_node, delete_node_by_id,
    delete_node_cascade, delete_node_links, delete_node_safe, delete_nodes_by_lab, get_node,
    get_node_by_id, get_node_by_name_and_lab, list_nodes, list_nodes_by_lab, update_node,
};

pub use schema::apply_schema;
pub use seed::node_config::delete_node_configs;
pub use seed::node_config::seed_node_configs;

// User CRUD operations
pub use user::{
    count_users, create_user, delete_user, delete_user_by_username, delete_user_safe, get_user,
    get_user_by_id, list_users, update_user, upsert_user,
};
