//! Phase 12: the lab's SSH config for the client.

use anyhow::{Context, Result};

use super::context::LabBuild;
use crate::services::ssh_config;

use shared::data;
use shared::data::StatusKind;
use shared::konst::SHERPA_SSH_PRIVATE_KEY_PATH;
use shared::util;

/// Write the lab's SSH config, with the server as `ProxyJump`, and return the server's private
/// key when the config uses it, or an empty string (phase 12). Node host keys are added once nodes are
/// ready, and the files saved again.
pub(super) fn write_ssh_files(
    build: &LabBuild<'_>,
    ztp_records: &[data::ZtpRecord],
    lab_info: &data::LabInfo,
) -> Result<String> {
    tracing::info!(lab_id = %build.lab_id, "Generating SSH configuration");
    // The server address comes from the running config, which applies SHERPA_SERVER_IPV4.
    let ssh_files = ssh_config::save_lab_ssh_files(
        build.lab_dir,
        ztp_records,
        lab_info,
        &[],
        build.config.server_ipv4,
    )?;
    let _ = build
        .progress
        .send_status("SSH config file created".to_string(), StatusKind::Done);
    let ssh_private_key = if ssh_files.uses_lab_key {
        util::load_file(SHERPA_SSH_PRIVATE_KEY_PATH).context("Failed to read SSH private key")?
    } else {
        String::new()
    };
    let _ = build
        .progress
        .send_status("SSH private key loaded".to_string(), StatusKind::Done);
    Ok(ssh_private_key)
}
