#![deny(clippy::unwrap_used, clippy::expect_used)]

mod connect;
mod container;
mod image;
mod network;

// Re-export connection utilities
pub use connect::docker_connection;

// Re-export container operations
pub use container::{
    exec_container, exec_container_detached, exec_container_with_retry, get_container_pid,
    kill_container, list_containers, pause_container, remove_container, run_container,
    start_container, stop_container, unpause_container,
};

// Re-export network operations
pub use network::{
    create_docker_bridge_network, create_docker_macvlan_bridge_network,
    create_docker_macvlan_network, delete_network, list_networks,
};

// Re-export image operations
pub use image::{
    get_local_images, list_images, load_image, pull_container_image, pull_image,
    save_container_image,
};

// Re-export Docker type for convenience
pub use bollard::Docker;
