use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Request type for destroying a lab
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DestroyRequest {
    pub lab_id: String,
    /// Username of the requesting user
    /// TODO: This is username-without-authentication. When adding authentication layer,
    /// replace this with verified identity from auth token/session.
    pub username: String,
}

/// Response from destroy operation with detailed tracking of all resources
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_destroy_request_serde_roundtrip() {
        let req = DestroyRequest {
            lab_id: "abc12345".to_string(),
            username: "admin".to_string(),
        };
        let json = serde_json::to_string(&req).expect("serializes");
        let back: DestroyRequest = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.lab_id, "abc12345");
        assert_eq!(back.username, "admin");
    }

    #[test]
    fn test_destroy_response_serde_roundtrip() {
        let resp = DestroyResponse {
            success: true,
            lab_id: "abc12345".to_string(),
            lab_name: "test-lab".to_string(),
            summary: DestroySummary {
                containers_destroyed: vec!["c1".to_string()],
                vms_destroyed: vec!["vm1".to_string()],
                ..Default::default()
            },
            errors: vec![],
        };
        let json = serde_json::to_string(&resp).expect("serializes");
        let back: DestroyResponse = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(back.success, true);
        assert_eq!(back.lab_id, "abc12345");
        assert_eq!(back.summary.containers_destroyed, vec!["c1"]);
        assert_eq!(back.summary.vms_destroyed, vec!["vm1"]);
        assert!(back.errors.is_empty());
    }

    #[test]
    fn test_destroy_error_new() {
        let err = DestroyError::new("vm", "vm01", "domain not found");
        assert_eq!(err.resource_type, "vm");
        assert_eq!(err.resource_name, "vm01");
        assert_eq!(err.error_message, "domain not found");
    }

    #[test]
    fn test_destroy_summary_default() {
        let summary = DestroySummary::default();
        assert!(summary.containers_destroyed.is_empty());
        assert!(summary.vms_destroyed.is_empty());
        assert!(!summary.lab_directory_deleted);
        assert!(!summary.database_records_deleted);
    }
}
