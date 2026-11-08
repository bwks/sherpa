//! Phase 12: the lab's SSH config for the client.

use anyhow::{Context, Result};

use super::context::LabBuild;
use crate::services::lab_ssh_key::LabSshKeyPaths;
use crate::services::ssh_config;

use shared::data;
use shared::data::StatusKind;
use shared::util;

/// Write the lab's SSH config, with the server as `ProxyJump`, and return the lab's private key
/// (`ssh_key`, made in phase 4) when the config uses it, or an empty string (phase 12). Node
/// host keys are added once nodes are ready, and the files saved again.
pub(super) fn write_ssh_files(
    build: &LabBuild<'_>,
    ztp_records: &[data::ZtpRecord],
    lab_info: &data::LabInfo,
    ssh_key: &LabSshKeyPaths,
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
        util::load_file(&ssh_key.private).context("Failed to read the lab's SSH private key")?
    } else {
        String::new()
    };
    let _ = build
        .progress
        .send_status("SSH private key loaded".to_string(), StatusKind::Done);
    Ok(ssh_private_key)
}
