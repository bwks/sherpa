use std::io::ErrorKind;

use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::DownloadLabResponse;
use shared::konst::{LAB_CA_CERT_FILE, LAB_CERTS_DIR, SHERPA_LABS_PATH};

use crate::auth::context::AuthContext;
use crate::auth::policy::require_lab_access;
use crate::daemon::state::AppState;
use crate::services::keycloak;
use crate::services::lab_info;
use crate::services::lab_ssh_key::{LabSshKeyPaths, read_private_key};
use crate::services::ssh_config::{self, LabSshFiles};
use crate::services::talos;
use crate::services::vault;

/// Download lab files (lab-info, SSH config, `known_hosts`, SSH key) for CLI use.
///
/// The lab info, SSH config and `known_hosts` are built from the database; the server keeps
/// no copy of them. The caller must own the lab or be an admin. The lab's SSH private key is
/// included only when the lab's SSH config uses it.
#[instrument(skip(ctx, state), fields(username = %ctx.username, %lab_id))]
pub async fn download_lab_files(
    ctx: &AuthContext,
    lab_id: &str,
    state: &AppState,
) -> Result<DownloadLabResponse> {
    let db_lab = require_lab_access(ctx, &state.db, lab_id).await?;
    let lab_record_id = db::get_lab_id(&db_lab).context("Failed to get lab record ID")?;

    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let lab_info = lab_info::load_lab_info(&state.db, &db_lab).await?;
    let LabSshFiles {
        ssh_config,
        known_hosts,
        uses_lab_key,
    } = ssh_config::lab_ssh_files(
        &state.db,
        &lab_record_id,
        &lab_info,
        state.config.server_ipv4,
    )
    .await?;

    let ssh_private_key = if uses_lab_key {
        read_private_key(&LabSshKeyPaths::in_lab(&lab_dir)).await?
    } else {
        String::new()
    };

    let clusters = talos::read_cluster_credentials(lab_id)?;

    // Labs created before the lab CA have no CA certificate.
    let lab_ca_path = format!("{lab_dir}/{LAB_CERTS_DIR}/{LAB_CA_CERT_FILE}");
    let lab_ca = match tokio::fs::read_to_string(&lab_ca_path).await {
        Ok(lab_ca) => lab_ca,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            tracing::debug!(path = %lab_ca_path, "Lab has no CA certificate");
            String::new()
        }
        Err(error) => {
            return Err(error).context(format!("Failed to read {lab_ca_path}"));
        }
    };
    let vaults = vault::read_vault_credentials(&vault::vault_dir(lab_id))?;
    let keycloaks = keycloak::read_keycloak_credentials(&keycloak::keycloak_dir(lab_id))?;

    Ok(DownloadLabResponse {
        lab_info,
        ssh_config,
        known_hosts,
        ssh_private_key,
        clusters,
        lab_ca,
        vaults,
        keycloaks,
    })
}
