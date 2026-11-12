#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod connect;
mod container;
mod image;
mod network;
mod status;

// Re-export connection utilities
pub use connect::docker_connection;

// Re-export container operations
pub use container::{
    ContainerSpec, container_log_tail, container_state, container_status, exec_container,
    exec_container_detached, exec_container_with_retry, get_container_mount_source,
    get_container_pid, kill_container, list_containers, pause_container, remove_container,
    run_container, start_container, stop_container, unpause_container,
};

// Re-export network operations
pub use network::{
    create_docker_bridge_network, create_docker_macvlan_bridge_network, delete_network,
    list_networks,
};

// Re-export image operations
pub use image::{get_local_images, image_exists, image_reference, load_image, pull_image};

// Re-export Docker type for convenience
pub use bollard::Docker;

pub mod tailscale;
