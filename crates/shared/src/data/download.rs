use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::LabInfo;

/// Response containing lab files for download
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadLabResponse {
    pub lab_info: LabInfo,
    pub ssh_config: String,
    pub ssh_private_key: String,
}
