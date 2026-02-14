use serde::{Deserialize, Serialize};

/// Request type for destroying a lab
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyRequest {
    pub lab_id: String,
    /// Username of the requesting user
    /// TODO: This is username-without-authentication. When adding authentication layer,
    /// replace this with verified identity from auth token/session.
    pub username: String,
}

/// Response from destroy operation with detailed tracking of all resources
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyResponse {
    /// Overall status: true if all succeeded, false if any failed
    pub success: bool,
    /// Lab ID that was destroyed
    pub lab_id: String,
    /// Lab name that was destroyed
    pub lab_name: String,
    /// Detailed summary of destroyed resources
    pub summary: DestroySummary,
    /// List of any errors that occurred
    pub errors: Vec<DestroyError>,
}

/// Detailed summary of all resources destroyed or failed
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DestroySummary {
    /// Containers successfully destroyed
    pub containers_destroyed: Vec<String>,
    /// Containers that failed to destroy
    pub containers_failed: Vec<String>,

    /// VMs successfully destroyed
    pub vms_destroyed: Vec<String>,
    /// VMs that failed to destroy
    pub vms_failed: Vec<String>,

    /// Disks successfully deleted
    pub disks_deleted: Vec<String>,
    /// Disks that failed to delete
    pub disks_failed: Vec<String>,

    /// Libvirt networks successfully destroyed
    pub libvirt_networks_destroyed: Vec<String>,
    /// Libvirt networks that failed to destroy
    pub libvirt_networks_failed: Vec<String>,

    /// Docker networks successfully destroyed
    pub docker_networks_destroyed: Vec<String>,
    /// Docker networks that failed to destroy
    pub docker_networks_failed: Vec<String>,

    /// Network interfaces successfully deleted
    pub interfaces_deleted: Vec<String>,
    /// Network interfaces that failed to delete
    pub interfaces_failed: Vec<String>,

    /// Whether lab directory was deleted
    pub lab_directory_deleted: bool,
    /// Whether database records were deleted
    pub database_records_deleted: bool,
}

/// Error that occurred during destroy operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestroyError {
    /// Type of resource (vm, container, disk, network, etc.)
    pub resource_type: String,
    /// Name of the resource that failed
    pub resource_name: String,
    /// Error message describing what went wrong
    pub error_message: String,
}

impl DestroyError {
    pub fn new(
        resource_type: impl Into<String>,
        resource_name: impl Into<String>,
        error_message: impl Into<String>,
    ) -> Self {
        Self {
            resource_type: resource_type.into(),
            resource_name: resource_name.into(),
            error_message: error_message.into(),
        }
    }
}
