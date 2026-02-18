mod connect;
mod container;
mod image;
mod network;

// Re-export connection utilities
pub use connect::docker_connection;

// Re-export container operations
pub use container::{kill_container, list_containers, remove_container, run_container};

// Re-export network operations
pub use network::{
    create_docker_bridge_network, create_docker_macvlan_network, delete_network, list_networks,
};

// Re-export image operations
pub use image::{
    get_local_images, list_images, pull_container_image, pull_image, save_container_image,
};

// Re-export Docker type for convenience
pub use bollard::Docker;
