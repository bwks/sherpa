use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use askama::Template;
use tracing::instrument;

use shared::data::{
    DbNode, LabInfo, NodeConfig, NodeHostKey, NodeSshUsername, RecordId, ZtpMethod, ZtpRecord,
};
use shared::konst::{
    LAB_FILE_NAME, SHERPA_DOMAIN_NAME, SHERPA_KNOWN_HOSTS_FILE, SHERPA_SSH_CONFIG_FILE,
    SHERPA_SSH_PRIVATE_KEY_FILE, SHERPA_USERNAME, SSH_PORT,
};
use shared::util;
use template::{KnownHostsEntry, KnownHostsTemplate, SshConfigTemplate, SshHostConfig};

/// A lab's SSH files, as saved in the lab directory and sent to the client.
pub(crate) struct LabSshFiles {
    pub ssh_config: String,
    pub known_hosts: String,
    /// Whether any node logs in with the shared lab key rather than a personal account.
    pub uses_lab_key: bool,
}

/// Record the host key `node` offered. Returns whether it replaced a different key, which
/// is logged as a warning.
#[instrument(skip(db, node, key), fields(node_name = %node.name))]
pub(crate) async fn record_node_host_key(
    db: &db::Database,
    node: &DbNode,
    key: &str,
) -> Result<bool> {
    if node.ssh_host_key.as_deref() == Some(key) {
        tracing::debug!("Host key unchanged");
        return Ok(false);
    }
    let node_id = node.id.clone().context("Node missing record ID")?;
    db::update_node_host_key(db, node_id, Some(key)).await?;
    let changed = node.ssh_host_key.is_some();
    if changed {
        tracing::warn!("Node host key changed; recorded the new key");
    } else {
        tracing::info!("Recorded node host key");
    }
    Ok(changed)
}

/// The lab info saved in `lab_dir`.
pub(crate) async fn load_lab_info(lab_dir: &str) -> Result<LabInfo> {
    tokio::fs::read_to_string(format!("{lab_dir}/{LAB_FILE_NAME}"))
        .await
        .context("Failed to read lab info")?
        .parse()
}

/// Save the lab's SSH files from the database: its nodes' addresses, whether their images
/// have SSH, and the host keys recorded for them, with `lab_info` for the SSH accounts.
#[instrument(skip(db, lab_info), fields(lab_id = %lab_info.id))]
pub(crate) async fn refresh_lab_ssh_files(
    db: &db::Database,
    lab_dir: &str,
    lab_record_id: &RecordId,
    lab_info: &LabInfo,
    server_ipv4: Ipv4Addr,
) -> Result<LabSshFiles> {
    let nodes = db::list_nodes_by_lab(db, lab_record_id.clone())
        .await
        .context("Failed to load nodes for the SSH configuration")?;
    let mut image_ids: Vec<RecordId> = Vec::with_capacity(nodes.len());
    for node in &nodes {
        if !image_ids.contains(&node.image) {
            image_ids.push(node.image.clone());
        }
    }
    let images = db::list_node_images_by_ids(db, image_ids)
        .await
        .context("Failed to load node images for the SSH configuration")?;
    let records = saved_node_ssh_records(&nodes, &images)?;
    save_lab_ssh_files(
        lab_dir,
        &records,
        lab_info,
        &node_host_keys(&nodes),
        server_ipv4,
    )
}

/// SSH records for a lab's saved nodes: every node with a management address, with SSH
/// unless its image's model has none.
pub(crate) fn saved_node_ssh_records(
    nodes: &[DbNode],
    images: &[NodeConfig],
) -> Result<Vec<ZtpRecord>> {
    let mut records = Vec::new();
    for node in nodes {
        let has_ssh = match images
            .iter()
            .find(|image| image.id.as_ref() == Some(&node.image))
        {
            Some(image) => image.model.has_ssh(),
            None => {
                tracing::debug!(node_name = %node.name, "Node image not found; keeping SSH");
                true
            }
        };
        let Some(ip) = &node.mgmt_ipv4 else {
            tracing::debug!(node_name = %node.name, "Node has no management address");
            continue;
        };
        records.push(ZtpRecord {
            node_name: node.name.clone(),
            ipv4_address: ip
                .parse()
                .context("Invalid saved node management address")?,
            ipv6_address: None,
            mac_address: String::new(),
            config_file: String::new(),
            ztp_method: ZtpMethod::None,
            ssh_port: has_ssh.then_some(SSH_PORT),
        });
    }
    Ok(records)
}

/// The host keys recorded for `nodes`.
pub(crate) fn node_host_keys(nodes: &[DbNode]) -> Vec<NodeHostKey> {
    let mut host_keys = Vec::new();
    for node in nodes {
        if let Some(key) = &node.ssh_host_key {
            host_keys.push(NodeHostKey {
                node_name: node.name.clone(),
                key: key.clone(),
            });
        }
    }
    host_keys
}

/// Render the lab's SSH config and `known_hosts` file, and save them with `lab_info` in
/// `lab_dir`.
#[instrument(skip(records, lab_info, host_keys), fields(lab_id = %lab_info.id))]
pub(crate) fn save_lab_ssh_files(
    lab_dir: &str,
    records: &[ZtpRecord],
    lab_info: &LabInfo,
    host_keys: &[NodeHostKey],
    server_ipv4: Ipv4Addr,
) -> Result<LabSshFiles> {
    let template = build_ssh_config(
        records,
        &lab_info.ssh_usernames,
        host_keys,
        &lab_info.id,
        &lab_info.user,
        server_ipv4,
    );
    let uses_lab_key = template
        .hosts
        .iter()
        .any(|host| host.identity_file.is_some());
    let ssh_config = template
        .render()
        .context("Failed to render the SSH configuration")?;
    let known_hosts = build_known_hosts(records, host_keys)
        .render()
        .context("Failed to render the known_hosts file")?;
    util::create_file(
        &format!("{lab_dir}/{SHERPA_SSH_CONFIG_FILE}"),
        ssh_config.clone(),
    )
    .context("Failed to save the SSH configuration")?;
    util::create_file(
        &format!("{lab_dir}/{SHERPA_KNOWN_HOSTS_FILE}"),
        known_hosts.clone(),
    )
    .context("Failed to save the known_hosts file")?;
    util::create_file(&format!("{lab_dir}/{LAB_FILE_NAME}"), lab_info.to_string())
        .context("Failed to save the lab info")?;
    tracing::info!(host_keys = host_keys.len(), "Saved lab SSH files");
    Ok(LabSshFiles {
        ssh_config,
        known_hosts,
        uses_lab_key,
    })
}

/// Resolve SSH identities and aliases before passing data to the renderer. Nodes with a
/// recorded host key are checked strictly against the lab's `known_hosts` file.
#[instrument(level = "debug", skip(records, ssh_usernames, host_keys))]
pub(crate) fn build_ssh_config(
    records: &[ZtpRecord],
    ssh_usernames: &[NodeSshUsername],
    host_keys: &[NodeHostKey],
    lab_id: &str,
    proxy_user: &str,
    server_ipv4: Ipv4Addr,
) -> SshConfigTemplate {
    let hosts = records
        .iter()
        .filter_map(|record| {
            let port = record.ssh_port?;
            let personal_username = ssh_usernames
                .iter()
                .find(|entry| entry.node_name == record.node_name)
                .map(|entry| entry.username.as_str());
            Some(SshHostConfig {
                aliases: format!(
                    "{} {}.{lab_id} {}.{lab_id}.{SHERPA_DOMAIN_NAME}",
                    record.ipv4_address, record.node_name, record.node_name
                ),
                hostname: record.ipv4_address,
                port,
                username: personal_username.unwrap_or(SHERPA_USERNAME).to_owned(),
                identity_file: personal_username
                    .is_none()
                    .then(|| SHERPA_SSH_PRIVATE_KEY_FILE.to_owned()),
                known_hosts_file: host_keys
                    .iter()
                    .any(|entry| entry.node_name == record.node_name)
                    .then(|| SHERPA_KNOWN_HOSTS_FILE.to_owned()),
            })
        })
        .collect();
    SshConfigTemplate {
        hosts,
        proxy_jump: format!("{proxy_user}@{server_ipv4}"),
    }
}

/// The lab's `known_hosts` file: the recorded host key of every node that has SSH.
#[instrument(level = "debug", skip(records, host_keys))]
pub(crate) fn build_known_hosts(
    records: &[ZtpRecord],
    host_keys: &[NodeHostKey],
) -> KnownHostsTemplate {
    let mut entries = Vec::new();
    for host_key in host_keys {
        let Some(record) = records
            .iter()
            .find(|record| record.node_name == host_key.node_name)
        else {
            tracing::debug!(node_name = %host_key.node_name, "host key for a node no longer in the lab");
            continue;
        };
        let Some(port) = record.ssh_port else {
            tracing::debug!(node_name = %host_key.node_name, "host key for a node without SSH");
            continue;
        };
        entries.push(KnownHostsEntry {
            hostname: record.ipv4_address,
            port,
            key: host_key.key.clone(),
        });
    }
    KnownHostsTemplate { entries }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{NodeModel, NodeState};

    fn records() -> Vec<ZtpRecord> {
        ["vm", "router"]
            .into_iter()
            .enumerate()
            .map(|(index, name)| ZtpRecord {
                node_name: name.to_owned(),
                ipv4_address: format!("192.0.2.{}", index + 10).parse().unwrap(),
                ipv6_address: None,
                config_file: String::new(),
                mac_address: String::new(),
                ztp_method: ZtpMethod::None,
                ssh_port: Some(if index == 0 { 22 } else { 830 }),
            })
            .collect()
    }

    #[test]
    fn mixed_lab_selects_personal_identity_and_preserves_legacy_options() {
        let usernames = [NodeSshUsername {
            node_name: "vm".to_owned(),
            username: "alice".to_owned(),
        }];
        let config = build_ssh_config(
            &records(),
            &usernames,
            &[],
            "abcd1234",
            "owner",
            "192.0.2.1".parse().unwrap(),
        );
        assert_eq!(config.hosts[0].username, "alice");
        assert!(config.hosts[0].identity_file.is_none());
        assert_eq!(config.hosts[1].username, SHERPA_USERNAME);
        assert_eq!(
            config.hosts[1].identity_file.as_deref(),
            Some(SHERPA_SSH_PRIVATE_KEY_FILE)
        );
        let output = config.render().unwrap();
        let (personal, legacy) = output.split_once("Host 192.0.2.11").unwrap();
        assert!(personal.contains("User alice"));
        assert!(personal.contains("vm.abcd1234.sherpa.lab.local"));
        assert!(personal.contains("ProxyJump owner@192.0.2.1"));
        assert!(!personal.contains("IdentityFile"));
        assert!(!personal.contains("IdentitiesOnly"));
        assert!(legacy.contains("User sherpa"));
        assert!(legacy.contains("Port 830"));
        assert!(legacy.contains("IdentityFile sherpa_ssh_key"));
        assert!(legacy.contains("IdentitiesOnly yes"));
    }

    #[test]
    fn unrelated_metadata_does_not_change_legacy_nodes() {
        let usernames = [NodeSshUsername {
            node_name: "other-vm".to_owned(),
            username: "alice".to_owned(),
        }];
        for usernames in [&[][..], &usernames[..]] {
            let config = build_ssh_config(
                &records(),
                usernames,
                &[],
                "abcd1234",
                "owner",
                "192.0.2.1".parse().unwrap(),
            );
            assert!(
                config
                    .hosts
                    .iter()
                    .all(|host| host.username == SHERPA_USERNAME
                        && host.identity_file.as_deref() == Some(SHERPA_SSH_PRIVATE_KEY_FILE))
            );
            assert_eq!(
                config.hosts[0].aliases,
                "192.0.2.10 vm.abcd1234 vm.abcd1234.sherpa.lab.local"
            );
            assert_eq!(config.proxy_jump, "owner@192.0.2.1");
        }
    }

    #[test]
    fn nodes_without_ssh_are_left_out() {
        let mut lab = records();
        lab.push(ZtpRecord {
            node_name: "talos".to_owned(),
            ipv4_address: "192.0.2.12".parse().unwrap(),
            ipv6_address: None,
            config_file: String::new(),
            mac_address: String::new(),
            ztp_method: ZtpMethod::NoCloud,
            ssh_port: None,
        });
        let config = build_ssh_config(
            &lab,
            &[],
            &[],
            "abcd1234",
            "owner",
            "192.0.2.1".parse().unwrap(),
        );
        assert_eq!(config.hosts.len(), 2);
        assert!(config.render().unwrap().find("talos").is_none());
    }

    fn host_key(node_name: &str, key: &str) -> NodeHostKey {
        NodeHostKey {
            node_name: node_name.to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn only_nodes_with_a_recorded_key_are_checked_strictly() {
        let keys = [host_key("router", "ssh-rsa AAAAB3")];
        let config = build_ssh_config(
            &records(),
            &[],
            &keys,
            "abcd1234",
            "owner",
            "192.0.2.1".parse().unwrap(),
        );
        assert!(config.hosts[0].known_hosts_file.is_none());
        assert_eq!(
            config.hosts[1].known_hosts_file.as_deref(),
            Some(SHERPA_KNOWN_HOSTS_FILE)
        );
    }

    #[test]
    fn known_hosts_lists_recorded_keys_by_ssh_address_and_port() {
        let keys = [
            host_key("vm", "ssh-ed25519 AAAAC3"),
            host_key("router", "ssh-rsa AAAAB3"),
            host_key("gone", "ssh-ed25519 AAAAC4"),
        ];
        let output = build_known_hosts(&records(), &keys).render().unwrap();
        assert_eq!(
            output,
            "192.0.2.10 ssh-ed25519 AAAAC3\n[192.0.2.11]:830 ssh-rsa AAAAB3\n"
        );
    }

    #[test]
    fn known_hosts_leaves_out_nodes_without_ssh() {
        let mut lab = records();
        lab[1].ssh_port = None;
        let keys = [host_key("router", "ssh-rsa AAAAB3")];
        assert_eq!(build_known_hosts(&lab, &keys).render().unwrap(), "");
    }

    fn db_node(name: &str, ip: Option<&str>) -> DbNode {
        DbNode {
            id: None,
            name: name.to_owned(),
            image: RecordId::new("node_image", "x"),
            index: 0,
            lab: RecordId::new("lab", "x"),
            mgmt_ipv4: ip.map(str::to_owned),
            mgmt_ipv6: None,
            mgmt_mac: None,
            state: NodeState::Running,
            cpu_count: None,
            memory: None,
            vnc_port: None,
            ssh_host_key: None,
            cpu_pinning: None,
            qemu_thread_cpus: None,
        }
    }

    #[test]
    fn saved_nodes_have_ssh_unless_their_image_has_none() {
        let mut ubuntu = NodeConfig::get_model(NodeModel::UbuntuLinux);
        ubuntu.id = Some(RecordId::new("node_image", "ubuntu"));
        let mut talos = NodeConfig::get_model(NodeModel::TalosLinux);
        talos.id = Some(RecordId::new("node_image", "talos"));
        let mut talos_node = db_node("talos", Some("192.0.2.11"));
        talos_node.image = RecordId::new("node_image", "talos");
        let mut vm = db_node("vm", Some("192.0.2.10"));
        vm.image = RecordId::new("node_image", "ubuntu");
        vm.ssh_host_key = Some("ssh-ed25519 AAAAC3".to_owned());
        let nodes = vec![
            vm,
            talos_node,
            db_node("unknown-image", Some("192.0.2.12")),
            db_node("no-address", None),
        ];
        let records = saved_node_ssh_records(&nodes, &[ubuntu, talos]).unwrap();
        let ports: Vec<(&str, Option<u16>)> = records
            .iter()
            .map(|record| (record.node_name.as_str(), record.ssh_port))
            .collect();
        assert_eq!(
            ports,
            [
                ("vm", Some(SSH_PORT)),
                ("talos", None),
                ("unknown-image", Some(SSH_PORT))
            ]
        );
        assert_eq!(
            node_host_keys(&nodes),
            [host_key("vm", "ssh-ed25519 AAAAC3")]
        );
    }

    fn saved(dir: &tempfile::TempDir, name: &str) -> String {
        std::fs::read_to_string(dir.path().join(name)).unwrap()
    }

    #[test]
    fn saved_lab_files_check_recorded_nodes_strictly() {
        let dir = tempfile::tempdir().unwrap();
        let lab_dir = dir.path().to_string_lossy().to_string();
        let lab_info: LabInfo = r#"
id = "abcd1234"
name = "lab"
user = "owner"
ipv4_network = "192.0.2.0/24"
ipv4_gateway = "192.0.2.1"
ipv4_router = "192.0.2.2"
loopback_network = "127.127.1.0/24"
"#
        .parse()
        .unwrap();
        let keys = [host_key("vm", "ssh-ed25519 AAAAC3")];
        let files = save_lab_ssh_files(
            &lab_dir,
            &records(),
            &lab_info,
            &keys,
            "192.0.2.1".parse().unwrap(),
        )
        .unwrap();
        assert!(files.uses_lab_key);
        assert_eq!(files.known_hosts, "192.0.2.10 ssh-ed25519 AAAAC3\n");
        let (vm, router) = files.ssh_config.split_once("Host 192.0.2.11").unwrap();
        assert!(vm.contains("StrictHostKeyChecking yes"));
        assert!(vm.contains("UserKnownHostsFile sherpa_known_hosts"));
        assert!(router.contains("StrictHostKeyChecking no"));
        assert_eq!(saved(&dir, SHERPA_SSH_CONFIG_FILE), files.ssh_config);
        assert_eq!(saved(&dir, SHERPA_KNOWN_HOSTS_FILE), files.known_hosts);
        let restored: LabInfo = saved(&dir, LAB_FILE_NAME).parse().unwrap();
        assert_eq!(restored.ssh_usernames, lab_info.ssh_usernames);
    }
}
