use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::{DownloadLabResponse, LabInfo};
use shared::konst::{
    LAB_FILE_NAME, SHERPA_LABS_PATH, SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE,
    SHERPA_SSH_PRIVATE_KEY_PATH,
};

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::talos;

/// Download lab files (lab-info, SSH config, SSH key) for CLI use.
///
/// The caller must own the lab or be an admin. The server SSH private key is included only
/// when the lab's SSH config uses it.
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn download_lab_files(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<DownloadLabResponse> {
    require_lab_access(ctx, &state.db, lab_id).await?;

    let lab_dir = format!("{}/{}", SHERPA_LABS_PATH, lab_id);
    let lab_info: LabInfo = tokio::fs::read_to_string(format!("{lab_dir}/{LAB_FILE_NAME}"))
        .await
        .context("Failed to read lab info")?
        .parse()?;

    let ssh_config_path = format!("{}/{}", lab_dir, SHERPA_SSH_CONFIG_FILE);
    let ssh_config = tokio::fs::read_to_string(&ssh_config_path)
        .await
        .context("SSH config not found. Has the lab been started?")?;

    let ssh_private_key = if ssh_config
        .lines()
        .any(|line| line.trim() == format!("IdentityFile {SHERPA_SSH_PRIVATE_KEY_FILE}"))
    {
        tokio::fs::read_to_string(SHERPA_SSH_PRIVATE_KEY_PATH)
            .await
            .context("SSH key not found on server")?
    } else {
        String::new()
    };

    let clusters = talos::read_cluster_credentials(lab_id)?;

    Ok(DownloadLabResponse {
        lab_info,
        ssh_config,
        ssh_private_key,
        clusters,
    })
}
