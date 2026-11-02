use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Response to a database backup: where the server wrote the copy.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackupResponse {
    /// Path of the backup file on the server.
    pub path: String,
    /// Size of the backup file in bytes.
    pub size_bytes: u64,
}
