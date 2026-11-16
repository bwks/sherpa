//! Phase 12: the lab's SSH key for the client.

use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use tracing::instrument;

use crate::services::lab_ssh_key::LabSshKeyPaths;
use crate::services::progress::ProgressSender;
use crate::services::ssh_config;

use shared::data;
use shared::data::StatusKind;
use shared::util;

/// The lab's private key (`ssh_key`, made in phase 4) when the lab's SSH config uses it, or an
/// empty string (phase 12). The SSH config, with the server at `server_ipv4` as `ProxyJump`, is
/// rendered in memory to tell; nothing is saved. The client gets the config and `known_hosts`
/// file built from the database once nodes are ready.
#[instrument(skip_all, fields(lab_id = %lab_info.id))]
pub(super) fn lab_private_key(
    progress: &ProgressSender,
    server_ipv4: Ipv4Addr,
    ztp_records: &[data::ZtpRecord],
    lab_info: &data::LabInfo,
    ssh_key: &LabSshKeyPaths,
) -> Result<String> {
    tracing::info!("Checking the lab's SSH configuration");
    let ssh_files = ssh_config::render_lab_ssh_files(ztp_records, lab_info, &[], server_ipv4)?;
    let _ = progress.send_status("SSH config generated".to_string(), StatusKind::Done);
    let ssh_private_key = if ssh_files.uses_lab_key {
        util::load_file(&ssh_key.private).context("Failed to read the lab's SSH private key")?
    } else {
        String::new()
    };
    let _ = progress.send_status("SSH private key loaded".to_string(), StatusKind::Done);
    Ok(ssh_private_key)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tokio::sync::mpsc;

    use crate::konst::SHERPA_SSH_PUBLIC_KEY_FILE;
    use shared::data::{NodeSshUsername, Ztp};
    use shared::konst::SHERPA_SSH_PRIVATE_KEY_FILE;

    use super::*;

    fn lab_info(ssh_usernames: Vec<NodeSshUsername>) -> data::LabInfo {
        data::LabInfo {
            ssh_usernames,
            id: "ab12cd34".to_owned(),
            name: "core".to_owned(),
            user: "alice".to_owned(),
            ipv4_network: "172.31.4.0/24".parse().unwrap(),
            ipv4_gateway: "172.31.4.1".parse().unwrap(),
            ipv4_router: "172.31.4.2".parse().unwrap(),
            loopback_network: "127.127.4.0/24".parse().unwrap(),
            ipv6_network: None,
            ipv6_gateway: None,
            ipv6_router: None,
            description: None,
            client_host: None,
            client_folder: None,
        }
    }

    fn records() -> Vec<data::ZtpRecord> {
        vec![data::ZtpRecord {
            node_name: "vm".to_owned(),
            ipv4_address: "172.31.4.10".parse().unwrap(),
            ipv6_address: None,
            mac_address: String::new(),
            config_file: String::new(),
            ztp: Ztp::Disabled,
            ssh_port: Some(22),
        }]
    }

    /// A lab directory as phase 4 leaves it: the lab's key pair and nothing else.
    fn lab_dir() -> (tempfile::TempDir, LabSshKeyPaths) {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(SHERPA_SSH_PRIVATE_KEY_FILE), "private key").unwrap();
        fs::write(root.path().join(SHERPA_SSH_PUBLIC_KEY_FILE), "ssh-rsa AAAA").unwrap();
        let paths = LabSshKeyPaths::in_lab(root.path().to_str().unwrap());
        (root, paths)
    }

    fn files_in(dir: &tempfile::TempDir) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn up_writes_no_lab_info_ssh_config_or_known_hosts() {
        let (root, paths) = lab_dir();
        let (tx, _rx) = mpsc::unbounded_channel();

        let key = lab_private_key(
            &ProgressSender::new(tx),
            "192.0.2.1".parse().unwrap(),
            &records(),
            &lab_info(Vec::new()),
            &paths,
        )
        .unwrap();

        assert_eq!(key, "private key");
        assert_eq!(
            files_in(&root),
            [SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_FILE]
        );
    }

    #[test]
    fn a_lab_whose_nodes_log_in_as_their_owner_gets_no_lab_key() {
        let (root, paths) = lab_dir();
        let (tx, _rx) = mpsc::unbounded_channel();
        let usernames = vec![NodeSshUsername {
            node_name: "vm".to_owned(),
            username: "alice".to_owned(),
        }];

        let key = lab_private_key(
            &ProgressSender::new(tx),
            "192.0.2.1".parse().unwrap(),
            &records(),
            &lab_info(usernames),
            &paths,
        )
        .unwrap();

        assert_eq!(key, "");
        assert_eq!(
            files_in(&root),
            [SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_SSH_PUBLIC_KEY_FILE]
        );
    }
}
