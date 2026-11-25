#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(not(test), forbid(unsafe_code))]

mod connect;
mod container;
mod error;
mod image;
#[cfg(test)]
mod mock_docker;
mod network;
mod node_vm;
mod status;

// Re-export connection utilities
pub use connect::docker_connection;

pub use error::ContainerError;

// Re-export container operations
pub use container::{
    ContainerMemory, ContainerSpec, container_log_tail, container_state, container_status,
    exec_container, exec_container_detached, exec_container_with_retry, get_container_mount_source,
    get_container_pid, kill_container, list_container_memory, list_containers, pause_container,
    remove_container, run_container, start_container, stop_container, unpause_container,
};

// Re-export network operations
pub use network::{
    BridgeSubnet, ExistingBridgeNetwork, NetworkDifference, create_docker_bridge_network,
    create_docker_macvlan_bridge_network, delete_network, inspect_existing_bridge_network,
    list_networks,
};

// Re-export image operations
pub use image::{
    get_local_images, image_exists, image_reference, load_image, pull_image, remove_image,
    running_containers_of_image,
};

// Re-export node VM operations
pub use node_vm::{
    NodeVmFile, NodeVmTls, copy_image_to_node_vm, ensure_node_vm_management_network,
    node_vm_docker, run_node_vm_container,
};

// Re-export Docker type for convenience
pub use bollard::Docker;

pub mod tailscale;
