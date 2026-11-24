use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Response to a database backup: where the server wrote the copy.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BackupResponse {
    pub path: String,
    pub size_bytes: u64,
}
