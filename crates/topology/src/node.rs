use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use serde_derive::{Deserialize, Serialize};

use shared::data::{
    ModelKind, NodeBootDisk, NodeCpu, NodeInterfaces, NodeKind, NodeModel, NodeNuma, PtxPlatform,
    WatchdogAction, Ztp,
};

use crate::provisioner::Provisioner;

/// A node in the manifest: `[[nodes]]`. The settings are grouped; `cpu`, `numa`, `boot_disk`,
/// `interfaces`, `ztp` and `ready_check` are shared by the kinds that take them, and
/// `vm`, `container` and `unikernel` hold the settings of one kind. Every setting is optional:
/// one left out keeps the image's value. Unknown keys, such as the flat keys of earlier
/// manifests, are refused.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub name: String,
    pub model: NodeModel,
    /// `virtual_machine`, `container` or `unikernel`: a kind the model takes. Absent, the node
    /// takes its model's default kind, the first of `NodeModel::kinds`.
    pub kind: Option<NodeKind>,
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

/// `[nodes.ready_check]`: how Sherpa decides the node is ready.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeReadyCheck {
    /// The TCP port that shows the node is ready: for a VM or unikernel, in place of the
    /// model's; for a container, once it has kept running.
    pub port: Option<u16>,
    /// Skip the check: the node counts as ready once started.
    pub skip: Option<bool>,
}

/// `[nodes.vm]`: settings only a virtual machine takes.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeVm {
    /// A hardware watchdog, and what QEMU does when it expires. Absent, the VM has none.
    pub watchdog: Option<WatchdogAction>,
}

/// `[nodes.container]`: settings only a container takes.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeContainer {
    pub user: Option<String>,
    pub privileged: Option<bool>,
    pub shm_size: Option<i64>,
    pub commands: Option<Vec<String>>,
    pub volumes: Option<Vec<VolumeMount>>,
    /// The PTX platform a `juniper_evolved` container emulates. Absent, it is BT.
    pub platform: Option<PtxPlatform>,
}

/// `[nodes.unikernel]`: settings only a unikernel takes.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct NodeUnikernel {
    /// The kernel command line.
    pub cmdline: Option<String>,
}

/// A manifest node as one node of its lab: its settings grouped as in the manifest, its
/// addresses, and the contents of the files and scripts the client read.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
pub struct NodeExpanded {
    pub index: u16,
    pub name: String,
    /// The node's model, and its kind: the manifest's `kind`, or else the model's default.
    pub model_kind: ModelKind,
    pub image: Option<String>,
    pub version: Option<String>,
    pub memory: Option<u16>,
    pub cpu: NodeCpu,
    pub numa: NodeNuma,
    pub boot_disk: NodeBootDisk,
    pub interfaces: NodeInterfaces,
    pub ztp: NodeZtp,
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
    /// The manifest `node` as the `index`th node of its lab, counting from 1. Its kind is the
    /// manifest's, or else its model's default. Its addresses are assigned later, and its
    /// scripts and files are the contents the client read.
    ///
    /// # Errors
    ///
    /// Fails when the node sets a `kind` its model does not take, which
    /// `validate::check_node_model_kind` reports first.
    pub fn new(index: u16, node: &Node) -> Result<Self> {
        let model_kind = ModelKind::new(node.model, node.kind)
            .with_context(|| format!("Manifest - node: '{}'", node.name))?;
        Ok(Self {
            index,
            name: node.name.clone(),
            model_kind,
            image: node.image.clone(),
            version: node.version.clone(),
            memory: node.memory,
            cpu: node.cpu.clone(),
            numa: node.numa.clone(),
            boot_disk: node.boot_disk.clone(),
            interfaces: node.interfaces.clone(),
            ztp: node.ztp.clone(),
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
        })
    }
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
pub struct StartupScript {
    pub filename: String,
    pub content: String,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VolumeMount {
    pub src: String,
    pub dst: String,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextFile {
    pub src: String,
    pub dst: String,
    pub user: String,
    pub group: String,
    pub permissions: u32,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
pub struct TextFileData {
    pub content: String,
    pub dst: String,
    pub user: String,
    pub group: String,
    pub permissions: u32,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BinaryFile {
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SystemdUnit {
    pub name: String,
    pub source: String,
    pub enabled: bool,
}

/// A binary file the client read: its file name and base64 contents.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
pub struct BinaryFileData {
    pub filename: String,
    pub content: String,
}

/// A systemd unit the client read: the unit name, its base64 contents and whether to enable it.
#[derive(Clone, Debug, Deserialize, Default, Serialize, PartialEq)]
pub struct SystemdUnitData {
    pub name: String,
    pub content: String,
    pub enabled: bool,
}
