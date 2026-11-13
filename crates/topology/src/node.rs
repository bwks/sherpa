use std::net::{Ipv4Addr, Ipv6Addr};

use serde_derive::{Deserialize, Serialize};

use shared::data::{
    NodeBootDisk, NodeCpu, NodeInterfaces, NodeModel, NodeNuma, PtxPlatform, WatchdogAction, Ztp,
};

use crate::provisioner::Provisioner;

/// A node in the manifest: `[[nodes]]`. The settings are grouped; `cpu`, `numa`, `boot_disk`,
/// `interfaces`, `ztp`, `ssh` and `ready_check` are shared by the kinds that take them, and
/// `vm`, `container` and `unikernel` hold the settings of one kind. Every setting is optional:
/// one left out keeps the image's value. Unknown keys, such as the flat keys of earlier
/// manifests, are refused.
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub name: String,
    pub model: NodeModel,
    pub image: Option<String>,
    pub version: Option<String>,
    /// The guest's memory in MiB, overriding the image's.
    pub memory: Option<u16>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub cpu: NodeCpu,
    #[serde(default, skip_serializing_if = "is_default")]
    pub numa: NodeNuma,
    #[serde(default, skip_serializing_if = "is_default")]
    pub boot_disk: NodeBootDisk,
    #[serde(default, skip_serializing_if = "is_default")]
    pub interfaces: NodeInterfaces,
    #[serde(default, skip_serializing_if = "is_default")]
    pub ztp: NodeZtp,
    #[serde(default, skip_serializing_if = "is_default")]
    pub ssh: NodeSsh,
    #[serde(default, skip_serializing_if = "is_default")]
    pub ready_check: NodeReadyCheck,
    /// Settings only a virtual machine takes.
    pub vm: Option<NodeVm>,
    /// Settings only a container takes.
    pub container: Option<NodeContainer>,
    /// Settings only a unikernel takes.
    pub unikernel: Option<NodeUnikernel>,
    pub environment_variables: Option<Vec<String>>,
    pub text_files: Option<Vec<TextFile>>,
    pub binary_files: Option<Vec<BinaryFile>>,
    pub systemd_units: Option<Vec<SystemdUnit>>,
    pub startup_scripts: Option<Vec<String>>,
    #[serde(default)]
    pub startup_scripts_data: Option<Vec<StartupScript>>,
    pub user_scripts: Option<Vec<String>>,
    #[serde(default)]
    pub user_scripts_data: Option<Vec<StartupScript>>,
    #[serde(default)]
    pub text_files_data: Option<Vec<TextFileData>>,
    /// The `binary_files` the client read, for the server.
    #[serde(default)]
    pub binary_files_data: Option<Vec<BinaryFileData>>,
    /// The `systemd_units` the client read, for the server.
    #[serde(default)]
    pub systemd_units_data: Option<Vec<SystemdUnitData>>,
    /// The keys in the `ssh.authorized_key_files` the client read, one OpenSSH line each.
    #[serde(default)]
    pub ssh_authorized_key_files_data: Option<Vec<String>>,
    /// Client-side provisioners, run by the CLI once the node is ready. Never serialized, so
    /// the client never sends them to the server and the server never stores them.
    #[serde(default, skip_serializing)]
    pub provisioners: Option<Vec<Provisioner>>,
}

/// Whether a group is left as its default, so a node that does not set it writes no table.
fn is_default<T: Default + PartialEq>(group: &T) -> bool {
    *group == T::default()
}

/// `[nodes.ztp]`: zero touch provisioning for the node.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeZtp {
    /// `disabled`, or a method the node's model takes. Absent, the node runs with its image's
    /// setting.
    pub method: Option<Ztp>,
    /// A file with the node's own startup configuration.
    pub config: Option<String>,
}

/// `[nodes.ssh]`: SSH keys the node's account accepts.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeSsh {
    pub authorized_keys: Option<Vec<String>>,
    pub authorized_key_files: Option<Vec<AuthorizedKeyFile>>,
}

/// `[nodes.ready_check]`: how Sherpa decides the node is ready.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeReadyCheck {
    /// The TCP port that shows a VM or unikernel is ready, overriding the model's.
    pub port: Option<u16>,
    /// Skip the check: the node counts as ready once started.
    pub skip: Option<bool>,
}

/// `[nodes.vm]`: settings only a virtual machine takes.
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeVm {
    /// A hardware watchdog, and what QEMU does when it expires. Absent, the VM has none.
    pub watchdog: Option<WatchdogAction>,
}

/// `[nodes.container]`: settings only a container takes.
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeContainer {
    pub user: Option<String>,
    pub privileged: Option<bool>,
    pub shm_size: Option<i64>,
    pub commands: Option<Vec<String>>,
    pub volumes: Option<Vec<VolumeMount>>,
    /// The PTX platform a `juniper_cevolved` node emulates. Absent, it is BT.
    pub platform: Option<PtxPlatform>,
}

/// `[nodes.unikernel]`: settings only a unikernel takes.
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeUnikernel {
    /// The kernel command line.
    pub cmdline: Option<String>,
}

/// A node's `ssh` group once the client has read its key files.
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct ExpandedSsh {
    pub authorized_keys: Option<Vec<String>>,
    /// The keys the client read from the manifest's `ssh.authorized_key_files`.
    pub authorized_key_files: Option<Vec<String>>,
}

/// A manifest node as one node of its lab: its settings grouped as in the manifest, its
/// addresses, and the contents of the files and scripts the client read.
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct NodeExpanded {
    pub index: u16,
    pub name: String,
    pub model: NodeModel,
    pub image: Option<String>,
    pub version: Option<String>,
    pub memory: Option<u16>,
    pub cpu: NodeCpu,
    pub numa: NodeNuma,
    pub boot_disk: NodeBootDisk,
    pub interfaces: NodeInterfaces,
    pub ztp: NodeZtp,
    pub ssh: ExpandedSsh,
    pub ready_check: NodeReadyCheck,
    pub vm: Option<NodeVm>,
    pub container: Option<NodeContainer>,
    pub unikernel: Option<NodeUnikernel>,
    pub ipv4_address: Option<Ipv4Addr>,
    pub ipv6_address: Option<Ipv6Addr>,
    pub environment_variables: Option<Vec<String>>,
    pub text_files: Option<Vec<TextFileData>>,
    pub binary_files: Option<Vec<BinaryFileData>>,
    pub systemd_units: Option<Vec<SystemdUnitData>>,
    pub startup_scripts: Option<Vec<StartupScript>>,
    pub user_scripts: Option<Vec<StartupScript>>,
}

impl NodeExpanded {
    /// The manifest `node` as the `index`th node of its lab, counting from 1. Its addresses are
    /// assigned later, and its scripts and files are the contents the client read.
    #[must_use]
    pub fn new(index: u16, node: &Node) -> Self {
        Self {
            index,
            name: node.name.clone(),
            model: node.model,
            image: node.image.clone(),
            version: node.version.clone(),
            memory: node.memory,
            cpu: node.cpu.clone(),
            numa: node.numa.clone(),
            boot_disk: node.boot_disk.clone(),
            interfaces: node.interfaces.clone(),
            ztp: node.ztp.clone(),
            ssh: ExpandedSsh {
                authorized_keys: node.ssh.authorized_keys.clone(),
                authorized_key_files: node.ssh_authorized_key_files_data.clone(),
            },
            ready_check: node.ready_check.clone(),
            vm: node.vm.clone(),
            container: node.container.clone(),
            unikernel: node.unikernel.clone(),
            ipv4_address: None,
            ipv6_address: None,
            environment_variables: node.environment_variables.clone(),
            text_files: node.text_files_data.clone(),
            binary_files: node.binary_files_data.clone(),
            systemd_units: node.systemd_units_data.clone(),
            startup_scripts: node.startup_scripts_data.clone(),
            user_scripts: node.user_scripts_data.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct StartupScript {
    pub filename: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VolumeMount {
    pub src: String,
    pub dst: String,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TextFile {
    pub src: String,
    pub dst: String,
    pub user: String,
    pub group: String,
    pub permissions: u32,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct TextFileData {
    pub content: String,
    pub dst: String,
    pub user: String,
    pub group: String,
    pub permissions: u32,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BinaryFile {
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SystemdUnit {
    pub name: String,
    pub source: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AuthorizedKeyFile {
    pub source: String,
}

/// A binary file the client read: its file name and base64 contents.
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct BinaryFileData {
    pub filename: String,
    pub content: String,
}

/// A systemd unit the client read: the unit name, its base64 contents and whether to enable it.
#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct SystemdUnitData {
    pub name: String,
    pub content: String,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_expanded_node_takes_the_contents_the_client_read() {
        let node = Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            cpu: NodeCpu {
                count: Some(4),
                ..NodeCpu::default()
            },
            startup_scripts: Some(vec!["setup.sh".to_string()]),
            startup_scripts_data: Some(vec![StartupScript {
                filename: "setup.sh".to_string(),
                content: "echo hi".to_string(),
            }]),
            text_files_data: Some(vec![TextFileData {
                content: "data".to_string(),
                dst: "/etc/motd".to_string(),
                ..TextFileData::default()
            }]),
            ..Node::default()
        };
        let expanded = NodeExpanded::new(3, &node);
        assert_eq!(expanded.index, 3);
        assert_eq!(expanded.name, "dev01");
        assert_eq!(expanded.cpu.count, Some(4));
        assert_eq!(expanded.startup_scripts.unwrap()[0].content, "echo hi");
        assert_eq!(expanded.text_files.unwrap()[0].dst, "/etc/motd");
        assert!(expanded.ipv4_address.is_none());
    }
}
