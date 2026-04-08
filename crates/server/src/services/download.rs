use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::{DownloadLabResponse, InspectRequest};
use shared::konst::{SHERPA_LABS_PATH, SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_PATH};

use crate::daemon::state::AppState;
use crate::services::inspect;

/// Download lab files (lab-info, SSH config, SSH key) for CLI use.
#[instrument(skip(state), fields(%lab_id, %username))]
pub async fn download_lab_files(
    lab_id: &str,
    username: &str,
    state: &AppState,
) -> Result<DownloadLabResponse> {
    // Use inspect to get lab info (handles ownership check via the caller)
    let request = InspectRequest {
        lab_id: lab_id.to_string(),
        username: username.to_string(),
    };
    let inspect_response = inspect::inspect_lab(request, state)
        .await
        .context("Failed to inspect lab")?;

    let lab_dir = format!("{}/{}", SHERPA_LABS_PATH, lab_id);

    let ssh_config_path = format!("{}/{}", lab_dir, SHERPA_SSH_CONFIG_FILE);
    let ssh_config = tokio::fs::read_to_string(&ssh_config_path)
        .await
        .context("SSH config not found. Has the lab been started?")?;

    let ssh_private_key = tokio::fs::read_to_string(SHERPA_SSH_PRIVATE_KEY_PATH)
        .await
        .context("SSH key not found on server")?;

    Ok(DownloadLabResponse {
        lab_info: inspect_response.lab_info.clone(),
        ssh_config,
        ssh_private_key,
    })
}
