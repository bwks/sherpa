use std::net::Ipv4Addr;

use anyhow::{Context, Result};
use askama::Template;
use futures::future::join_all;
use tracing::instrument;

use shared::data::{
    DbNode, DbVmSandbox, LabInfo, ModelKind, NodeConfig, NodeHostKey, NodeSshUsername, RecordId,
    Ztp, ZtpRecord,
};
use shared::konst::{SANDBOX_HOST_SUFFIX, SHERPA_DOMAIN_NAME, SHERPA_KNOWN_HOSTS_FILE, SSH_PORT};
use template::{
    KnownHostsEntry, KnownHostsTemplate, SANDBOX_LOGIN, SshConfigTemplate, SshHostConfig,
};

use crate::services::node_ops;

/// A lab's SSH files, as sent to the client. They are never saved on the server.
pub(crate) struct LabSshFiles {
    pub ssh_config: String,
    pub known_hosts: String,
}

/// A sandbox's own SSH host: the name of its node, the sandbox's own IPv4 address and the host
/// key recorded for it.
pub(crate) struct SandboxSshHost {
    pub node_name: String,
    pub ipv4: Ipv4Addr,
    pub host_key: Option<String>,
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

/// Read the host key the sandbox's own sshd offers on port 22 of the sandbox's own IPv4 address,
/// as a node's is read (`node_ops::check_node_ssh_ready`), and record it in the sandbox's record when it
/// is not the one recorded there. A sandbox whose sshd does not answer keeps the key it has; the
/// scanner reads it later. A key that replaced a different one is logged as a warning.
#[instrument(skip(db, sandbox), fields(node = %sandbox.node, ipv4 = %sandbox.ipv4), level = "debug")]
pub(crate) async fn capture_sandbox_host_key(
    db: &db::Database,
    sandbox: &DbVmSandbox,
) -> Result<()> {
    let Some(key) = node_ops::check_node_ssh_ready(&sandbox.ipv4.to_string(), SSH_PORT).await?
    else {
        tracing::debug!("Sandbox sshd not answering; keeping the recorded key");
        return Ok(());
    };
    if sandbox.ssh_host_key.as_deref() == Some(key.as_str()) {
        tracing::debug!("Sandbox host key unchanged");
        return Ok(());
    }
    db::update_vm_sandbox_host_key(db, &sandbox.node, Some(&key)).await?;
    if sandbox.ssh_host_key.is_some() {
        tracing::warn!("Sandbox host key changed; recorded the new key");
    } else {
        tracing::info!("Recorded sandbox host key");
    }
    Ok(())
}

/// Read and record the host keys of the sandboxes recorded in `vm_sandboxes`
/// (`capture_sandbox_host_key`), all of them at once, so sandboxes whose sshd does not answer add
/// one probe's wait, not one each. A key that cannot be recorded is logged, and the others are still recorded.
#[instrument(skip_all, fields(sandboxes = vm_sandboxes.len()), level = "debug")]
pub(crate) async fn capture_sandbox_host_keys(db: &db::Database, vm_sandboxes: &[&DbVmSandbox]) {
    probe_sandboxes(vm_sandboxes, |sandbox| {
        capture_sandbox_host_key(db, sandbox)
    })
    .await;
}

/// Run `probe` on every sandbox in `vm_sandboxes` at once, and log each that fails.
async fn probe_sandboxes<'a, F, Fut>(vm_sandboxes: &[&'a DbVmSandbox], probe: F)
where
    F: Fn(&'a DbVmSandbox) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let results = join_all(vm_sandboxes.iter().map(|sandbox| probe(sandbox))).await;
    for (sandbox, result) in vm_sandboxes.iter().zip(results) {
        if let Err(error) = result {
            tracing::warn!(node = %sandbox.node, error = %format!("{error:#}"), "Failed to record the sandbox host key");
        }
    }
}

/// The lab's SSH config and `known_hosts` file, built from the database: its nodes'
/// addresses, whether their images have SSH, the host keys recorded for them and the sandboxes'
/// own addresses, with `lab_info` for the SSH accounts. Nothing is saved: the client gets them from `up` and
/// `download`. Every node is reached directly; a jump through the server is the client's to
/// add (`--ssh-jump`, or `ssh -J`).
#[instrument(skip(db, lab_info), fields(lab_id = %lab_info.id), level = "debug")]
pub(crate) async fn lab_ssh_files(
    db: &db::Database,
    lab_record_id: &RecordId,
    lab_info: &LabInfo,
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
    let vm_sandboxes = db::list_vm_sandboxes(db, lab_record_id)
        .await
        .context("Failed to load sandbox records for the SSH configuration")?;
    let sandboxes = sandbox_ssh_hosts(&nodes, &vm_sandboxes);
    render_lab_ssh_files(&records, &sandboxes, lab_info, &node_host_keys(&nodes))
}

/// The SSH hosts of the sandboxes recorded in `vm_sandboxes`, named by their nodes in `nodes`,
/// in the order of `vm_sandboxes`.
fn sandbox_ssh_hosts(nodes: &[DbNode], vm_sandboxes: &[DbVmSandbox]) -> Vec<SandboxSshHost> {
    vm_sandboxes
        .iter()
        .filter_map(|sandbox| {
            let node = nodes
                .iter()
                .find(|node| node.id.as_ref() == Some(&sandbox.node));
            if node.is_none() {
                tracing::debug!(node = %sandbox.node, "Sandbox record of a node not in the lab");
            }
            node.map(|node| SandboxSshHost {
                node_name: node.name.clone(),
                ipv4: sandbox.ipv4,
                host_key: sandbox.ssh_host_key.clone(),
            })
        })
        .collect()
}

/// SSH records for a lab's saved nodes: every node with a management address, with SSH
/// unless its image's model has none.
pub(crate) fn saved_node_ssh_records(
    nodes: &[DbNode],
    images: &[NodeConfig],
) -> Result<Vec<ZtpRecord>> {
    let mut records = Vec::new();
    for node in nodes {
        let has_ssh = if let Some(image) = images
            .iter()
            .find(|image| image.id.as_ref() == Some(&node.image))
        {
            image.model_kind().is_ok_and(ModelKind::has_ssh)
        } else {
            tracing::debug!(node_name = %node.name, "Node image not found; keeping SSH");
            true
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
            ztp: Ztp::Disabled,
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

/// Render the lab's SSH config and `known_hosts` file.
pub(crate) fn render_lab_ssh_files(
    records: &[ZtpRecord],
    sandboxes: &[SandboxSshHost],
    lab_info: &LabInfo,
    host_keys: &[NodeHostKey],
) -> Result<LabSshFiles> {
    let template = build_ssh_config(
        records,
        sandboxes,
        &lab_info.ssh_usernames,
        host_keys,
        &lab_info.id,
        &lab_info.user,
    );
    let ssh_config = template
        .render()
        .context("Failed to render the SSH configuration")?;
    let known_hosts = build_known_hosts(records, sandboxes, host_keys)
        .render()
        .context("Failed to render the known_hosts file")?;
    Ok(LabSshFiles {
        ssh_config,
        known_hosts,
    })
}

/// Resolve each node's login and aliases before passing data to the renderer. A node logs in
/// with its recorded login, else as the lab's owner, `owner`, and is reached directly, with
/// no jump host. No node names an identity file: each logs in with the owner's own keys. Nodes with
/// a recorded host key are checked strictly against the lab's `known_hosts` file. Each sandbox in
/// `sandboxes` follows as `<node>-sbx`, logging in as `core` on its own address, checked strictly
/// as a node is once its host key is recorded.
#[instrument(level = "debug", skip(records, sandboxes, ssh_usernames, host_keys))]
pub(crate) fn build_ssh_config(
    records: &[ZtpRecord],
    sandboxes: &[SandboxSshHost],
    ssh_usernames: &[NodeSshUsername],
    host_keys: &[NodeHostKey],
    lab_id: &str,
    owner: &str,
) -> SshConfigTemplate {
    let hosts = records
        .iter()
        .filter_map(|record| {
            let port = record.ssh_port?;
            let login = ssh_usernames
                .iter()
                .find(|entry| entry.node_name == record.node_name)
                .map_or(owner, |entry| entry.username.as_str());
            Some(SshHostConfig {
                aliases: format!(
                    "{} {}.{lab_id} {}.{lab_id}.{SHERPA_DOMAIN_NAME}",
                    record.ipv4_address, record.node_name, record.node_name
                ),
                hostname: record.ipv4_address,
                port,
                username: login.to_owned(),
                known_hosts_file: host_keys
                    .iter()
                    .any(|entry| entry.node_name == record.node_name)
                    .then(|| SHERPA_KNOWN_HOSTS_FILE.to_owned()),
            })
        })
        .chain(sandboxes.iter().map(|sandbox| {
            let name = format!("{}{SANDBOX_HOST_SUFFIX}", sandbox.node_name);
            SshHostConfig {
                aliases: format!(
                    "{} {name}.{lab_id} {name}.{lab_id}.{SHERPA_DOMAIN_NAME}",
                    sandbox.ipv4
                ),
                hostname: sandbox.ipv4,
                port: SSH_PORT,
                username: SANDBOX_LOGIN.to_owned(),
                known_hosts_file: sandbox
                    .host_key
                    .is_some()
                    .then(|| SHERPA_KNOWN_HOSTS_FILE.to_owned()),
            }
        }))
        .collect();
    SshConfigTemplate { hosts }
}

/// The lab's `known_hosts` file: the recorded host key of every node that has SSH, then of every
/// sandbox in `sandboxes`, each named by its own IPv4 address.
#[instrument(level = "debug", skip(records, sandboxes, host_keys))]
pub(crate) fn build_known_hosts(
    records: &[ZtpRecord],
    sandboxes: &[SandboxSshHost],
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
        if record.ssh_port.is_none() {
            tracing::debug!(node_name = %host_key.node_name, "host key for a node without SSH");
            continue;
        }
        entries.push(KnownHostsEntry {
            hostname: record.ipv4_address,
            key: host_key.key.clone(),
        });
    }
    entries.extend(sandboxes.iter().filter_map(|sandbox| {
        sandbox.host_key.as_ref().map(|key| KnownHostsEntry {
            hostname: sandbox.ipv4,
            key: key.clone(),
        })
    }));
    KnownHostsTemplate { entries }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use tokio::time::sleep;

    use super::*;
    use shared::data::ReadyCheck;
    use shared::data::{NodeModel, NodeState};

    use crate::services::lab_definition::tests::lab_with;
    use crate::services::lab_info;

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
                ztp: Ztp::Disabled,
                ssh_port: Some(if index == 0 { 22 } else { 830 }),
            })
            .collect()
    }

    #[test]
    fn ssh_config_hosts_003_each_node_has_one_host_logging_in_with_its_login_or_as_the_owner() {
        let usernames = [NodeSshUsername {
            node_name: "router".to_owned(),
            username: "admin".to_owned(),
        }];
        let config = build_ssh_config(&records(), &[], &usernames, &[], "abcd1234", "alice");
        assert_eq!(config.hosts[0].username, "alice");
        assert_eq!(config.hosts[1].username, "admin");
        assert_eq!(
            config.hosts[0].aliases,
            "192.0.2.10 vm.abcd1234 vm.abcd1234.sherpa.lab.local"
        );
        let output = config.render().unwrap();
        let (vm, router) = output.split_once("Host 192.0.2.11").unwrap();
        assert!(vm.contains("User alice"));
        assert!(router.contains("User admin"));
        assert!(router.contains("Port 830"));
        assert!(!output.contains("IdentityFile"));
        assert!(!output.contains("IdentitiesOnly"));
    }

    fn host_key(node_name: &str, key: &str) -> NodeHostKey {
        NodeHostKey {
            node_name: node_name.to_owned(),
            key: key.to_owned(),
        }
    }

    #[test]
    fn node_host_keys_017_a_key_is_named_by_its_node_s_address() {
        let keys = [
            host_key("vm", "ssh-ed25519 AAAAC3"),
            host_key("router", "ssh-rsa AAAAB3"),
        ];
        let output = build_known_hosts(&records(), &[], &keys).render().unwrap();
        assert_eq!(
            output,
            "192.0.2.10 ssh-ed25519 AAAAC3\n192.0.2.11 ssh-rsa AAAAB3\n"
        );
    }

    #[test]
    fn node_host_keys_006_keys_of_nodes_gone_or_without_ssh_are_left_out() {
        let mut lab = records();
        lab[1].ssh_port = None;
        let keys = [
            host_key("router", "ssh-rsa AAAAB3"),
            host_key("gone", "ssh-ed25519 AAAAC4"),
        ];
        assert_eq!(build_known_hosts(&lab, &[], &keys).render().unwrap(), "");
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
            ssh_host_key: None,
            ready_check: ReadyCheck::default(),
            ssh_username: None,
        }
    }

    #[test]
    fn ssh_config_hosts_004_talos_and_nodes_without_an_address_have_no_host() {
        let mut ubuntu = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        ubuntu.id = Some(RecordId::new("node_image", "ubuntu"));
        let mut talos = ModelKind::default_for(NodeModel::TalosLinux).node_config();
        talos.id = Some(RecordId::new("node_image", "talos"));
        let mut talos_node = db_node("talos", Some("192.0.2.11"));
        talos_node.image = RecordId::new("node_image", "talos");
        let mut vm = db_node("vm", Some("192.0.2.10"));
        vm.image = RecordId::new("node_image", "ubuntu");
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
    }

    /// One host of the SSH config as the template writes it.
    fn host(aliases: &str, address: &str, user: &str, checked: bool) -> String {
        let mut host =
            format!("\nHost {aliases}\n    HostName {address}\n    Port 22\n    User {user}");
        if checked {
            host.push_str(
                "\n    StrictHostKeyChecking yes\n    UserKnownHostsFile sherpa_known_hosts",
            );
        } else {
            host.push_str("\n    StrictHostKeyChecking no\n    UserKnownHostsFile /dev/null");
        }
        host.push_str(
            "\n    PubkeyAcceptedAlgorithms +ssh-rsa\n    HostkeyAlgorithms +ssh-rsa\n    \
             KexAlgorithms +diffie-hellman-group-exchange-sha1,diffie-hellman-group14-sha1",
        );
        host
    }

    #[tokio::test]
    async fn ssh_config_hosts_005_the_ssh_files_are_built_from_the_lab_s_records() {
        let (db, lab) = lab_with(&[
            ("vm1", NodeModel::AlpineLinux),
            ("vm2", NodeModel::UbuntuLinux),
            ("web", NodeModel::PostgresqlDb),
            ("talos", NodeModel::TalosLinux),
        ])
        .await;
        let lab_record_id = lab.id.clone().unwrap();
        let nodes = db::list_nodes_by_lab(&db, lab_record_id.clone())
            .await
            .unwrap();
        for (name, address) in [
            ("talos", "172.31.1.13"),
            ("vm1", "172.31.1.10"),
            ("vm2", "172.31.1.11"),
            ("web", "172.31.1.12"),
        ] {
            let node = nodes.iter().find(|node| node.name == name).unwrap();
            db::update_node_mgmt_ipv4(&db, node.id.clone().unwrap(), address)
                .await
                .unwrap();
        }
        let vm1 = nodes.iter().find(|node| node.name == "vm1").unwrap();
        db::update_node_host_key(&db, vm1.id.clone().unwrap(), Some("ssh-ed25519 AAAAC3vm1"))
            .await
            .unwrap();
        db::update_node_ssh_username(&db, vm1.id.clone().unwrap(), Some("alice"))
            .await
            .unwrap();
        // A node migration 29 recorded as `sherpa`.
        let vm2 = nodes.iter().find(|node| node.name == "vm2").unwrap();
        db::update_node_ssh_username(&db, vm2.id.clone().unwrap(), Some("sherpa"))
            .await
            .unwrap();
        let lab_info = lab_info::load_lab_info(&db, &lab).await.unwrap();

        let files = lab_ssh_files(&db, &lab_record_id, &lab_info).await.unwrap();

        // Nodes in name order; Talos Linux has no SSH, so it is left out. `web` has no recorded
        // login, so it logs in as the owner. No node names an identity file.
        let expected_config = [
            host(
                "172.31.1.10 vm1.lab-0001 vm1.lab-0001.sherpa.lab.local",
                "172.31.1.10",
                "alice",
                true,
            ),
            host(
                "172.31.1.11 vm2.lab-0001 vm2.lab-0001.sherpa.lab.local",
                "172.31.1.11",
                "sherpa",
                false,
            ),
            host(
                "172.31.1.12 web.lab-0001 web.lab-0001.sherpa.lab.local",
                "172.31.1.12",
                "alice",
                false,
            ),
        ]
        .concat();
        assert_eq!(files.ssh_config, expected_config);
        assert_eq!(files.known_hosts, "172.31.1.10 ssh-ed25519 AAAAC3vm1\n");
    }

    #[tokio::test]
    async fn ssh_config_hosts_010_a_sandbox_has_a_host_of_its_own_logging_in_as_core() {
        let (db, lab) = lab_with(&[
            ("srl1", NodeModel::NokiaSrlinux),
            ("vm1", NodeModel::AlpineLinux),
        ])
        .await;
        let lab_record_id = lab.id.clone().unwrap();
        let nodes = db::list_nodes_by_lab(&db, lab_record_id.clone())
            .await
            .unwrap();
        for (name, address) in [("srl1", "172.31.1.10"), ("vm1", "172.31.1.11")] {
            let node = nodes.iter().find(|node| node.name == name).unwrap();
            db::update_node_mgmt_ipv4(&db, node.id.clone().unwrap(), address)
                .await
                .unwrap();
        }
        let srl1 = nodes.iter().find(|node| node.name == "srl1").unwrap();
        db::update_node_host_key(
            &db,
            srl1.id.clone().unwrap(),
            Some("ssh-ed25519 AAAAC3srl1"),
        )
        .await
        .unwrap();
        db::create_vm_sandbox(
            &db,
            &DbVmSandbox {
                node: srl1.id.clone().unwrap(),
                ipv4: "172.31.1.12".parse().unwrap(),
                ipv6: Some("fd00::c".parse().unwrap()),
                mac: "52:54:00:00:00:09".to_owned(),
                ssh_host_key: None,
            },
        )
        .await
        .unwrap();
        let lab_info = lab_info::load_lab_info(&db, &lab).await.unwrap();

        let files = lab_ssh_files(&db, &lab_record_id, &lab_info).await.unwrap();

        let expected_config = [
            host(
                "172.31.1.10 srl1.lab-0001 srl1.lab-0001.sherpa.lab.local",
                "172.31.1.10",
                "alice",
                true,
            ),
            host(
                "172.31.1.11 vm1.lab-0001 vm1.lab-0001.sherpa.lab.local",
                "172.31.1.11",
                "alice",
                false,
            ),
            host(
                "172.31.1.12 srl1-sbx.lab-0001 srl1-sbx.lab-0001.sherpa.lab.local",
                "172.31.1.12",
                "core",
                false,
            ),
        ]
        .concat();
        assert_eq!(files.ssh_config, expected_config);
        assert_eq!(files.known_hosts, "172.31.1.10 ssh-ed25519 AAAAC3srl1\n");
    }

    #[tokio::test]
    async fn node_host_keys_018_a_sandbox_s_recorded_key_is_named_by_its_own_address_and_checked() {
        let (db, lab) = lab_with(&[
            ("srl1", NodeModel::NokiaSrlinux),
            ("srl2", NodeModel::NokiaSrlinux),
        ])
        .await;
        let lab_record_id = lab.id.clone().unwrap();
        let nodes = db::list_nodes_by_lab(&db, lab_record_id.clone())
            .await
            .unwrap();
        for (index, name) in ["srl1", "srl2"].into_iter().enumerate() {
            let node = nodes.iter().find(|node| node.name == name).unwrap();
            let node_id = node.id.clone().unwrap();
            db::update_node_mgmt_ipv4(&db, node_id.clone(), &format!("172.31.1.{}", 10 + index))
                .await
                .unwrap();
            db::update_node_host_key(
                &db,
                node_id.clone(),
                Some(&format!("ssh-ed25519 AAAA{name}")),
            )
            .await
            .unwrap();
            db::create_vm_sandbox(
                &db,
                &DbVmSandbox {
                    node: node_id,
                    ipv4: format!("172.31.1.{}", 12 + index).parse().unwrap(),
                    ipv6: None,
                    mac: format!("52:54:00:00:00:0{index}"),
                    ssh_host_key: None,
                },
            )
            .await
            .unwrap();
        }
        let srl1 = nodes.iter().find(|node| node.name == "srl1").unwrap();
        db::update_vm_sandbox_host_key(
            &db,
            srl1.id.as_ref().unwrap(),
            Some("ssh-ed25519 AAAAsbx1"),
        )
        .await
        .unwrap();
        let lab_info = lab_info::load_lab_info(&db, &lab).await.unwrap();

        let files = lab_ssh_files(&db, &lab_record_id, &lab_info).await.unwrap();

        // srl2's sandbox has no key recorded yet, so its host is not checked.
        let expected_sandboxes = [
            host(
                "172.31.1.12 srl1-sbx.lab-0001 srl1-sbx.lab-0001.sherpa.lab.local",
                "172.31.1.12",
                "core",
                true,
            ),
            host(
                "172.31.1.13 srl2-sbx.lab-0001 srl2-sbx.lab-0001.sherpa.lab.local",
                "172.31.1.13",
                "core",
                false,
            ),
        ]
        .concat();
        assert!(files.ssh_config.ends_with(&expected_sandboxes));
        assert_eq!(
            files.known_hosts,
            "172.31.1.10 ssh-ed25519 AAAAsrl1\n172.31.1.11 ssh-ed25519 AAAAsrl2\n\
             172.31.1.12 ssh-ed25519 AAAAsbx1\n"
        );
    }

    #[tokio::test]
    async fn sandbox_097_sandboxes_host_keys_are_read_at_once_and_one_failure_leaves_the_others() {
        const PROBE: Duration = Duration::from_millis(400);
        let vm_sandboxes: Vec<DbVmSandbox> = (0..4u8)
            .map(|index| DbVmSandbox {
                node: RecordId::new("node", format!("sbx{index}")),
                ipv4: Ipv4Addr::new(172, 31, 1, 20 + index),
                ipv6: None,
                mac: format!("52:54:00:00:00:0{index}"),
                ssh_host_key: None,
            })
            .collect();
        let refs: Vec<&DbVmSandbox> = vm_sandboxes.iter().collect();
        let recorded = Mutex::new(Vec::new());

        let started = Instant::now();
        probe_sandboxes(&refs, |sandbox| {
            let recorded = &recorded;
            async move {
                sleep(PROBE).await;
                if sandbox.ipv4 == Ipv4Addr::new(172, 31, 1, 21) {
                    anyhow::bail!("database refused the key");
                }
                recorded.lock().unwrap().push(sandbox.ipv4);
                Ok(())
            }
        })
        .await;
        let elapsed = started.elapsed();

        assert!(elapsed < PROBE * 2, "probes took {elapsed:?}");
        let mut recorded = recorded.into_inner().unwrap();
        recorded.sort();
        assert_eq!(
            recorded,
            [20, 22, 23].map(|host| Ipv4Addr::new(172, 31, 1, host))
        );
    }
}
