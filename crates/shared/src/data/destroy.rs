use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request type for destroying a lab
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DestroyRequest {
    pub lab_id: String,
}

/// Response from destroy operation with detailed tracking of all resources
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DestroyResponse {
    pub success: bool,
    pub lab_id: String,
    pub lab_name: String,
    pub summary: DestroySummary,
    pub errors: Vec<DestroyError>,
}

/// Detailed summary of all resources destroyed or failed
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DestroySummary {
    pub containers_destroyed: Vec<String>,
    pub containers_failed: Vec<String>,

    pub vms_destroyed: Vec<String>,
    pub vms_failed: Vec<String>,

    pub disks_deleted: Vec<String>,
    pub disks_failed: Vec<String>,

    pub libvirt_networks_destroyed: Vec<String>,
    pub libvirt_networks_failed: Vec<String>,

    pub docker_networks_destroyed: Vec<String>,
    pub docker_networks_failed: Vec<String>,

    pub interfaces_deleted: Vec<String>,
    pub interfaces_failed: Vec<String>,

    pub lab_directory_deleted: bool,
    pub database_records_deleted: bool,
}

/// Error that occurred during destroy operation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DestroyError {
    pub resource_type: String,
    pub resource_name: String,
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
