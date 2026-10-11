use anyhow::{Context, Result, ensure};
use tracing::instrument;

use shared::data::{DownloadLabResponse, LabInfo};
use shared::konst::{
    LAB_FILE_NAME, SHERPA_LABS_PATH, SHERPA_SSH_CONFIG_FILE, SHERPA_SSH_PRIVATE_KEY_FILE,
    SHERPA_SSH_PRIVATE_KEY_PATH,
};

use crate::daemon::state::AppState;

/// Download lab files (lab-info, SSH config, SSH key) for CLI use.
#[instrument(skip(state), fields(%lab_id, %username))]
pub async fn download_lab_files(
    lab_id: &str,
    username: &str,
    state: &AppState,
) -> Result<DownloadLabResponse> {
    let user = db::get_user(&state.db, username)
        .await
        .context("Failed to get authenticated user")?;
    let lab = db::get_lab(&state.db, lab_id)
        .await
        .context("Failed to get lab")?;
    ensure!(
        user.is_admin || user.id.as_ref() == Some(&lab.user),
        "Permission denied: lab belongs to another user"
    );

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

    Ok(DownloadLabResponse {
        lab_info,
        ssh_config,
        ssh_private_key,
    })
}
