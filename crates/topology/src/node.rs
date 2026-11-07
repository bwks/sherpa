use std::net::{Ipv4Addr, Ipv6Addr};

use serde_derive::{Deserialize, Serialize};

use shared::data::{
    DiskImageCloneMethod, NodeModel, PerformanceTuning, PtxPlatform, WatchdogAction,
};

use crate::provisioner::Provisioner;

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub name: String,
    pub model: NodeModel,
    pub image: Option<String>,
    pub version: Option<String>,
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
    pub boot_disk_size: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_disk_clone_method: Option<DiskImageCloneMethod>,
    pub data_interface_count: Option<u8>,
    pub text_files: Option<Vec<TextFile>>,
    pub binary_files: Option<Vec<BinaryFile>>,
    pub systemd_units: Option<Vec<SystemdUnit>>,
    pub ssh_authorized_keys: Option<Vec<String>>,
    pub ssh_authorized_key_files: Option<Vec<AuthorizedKeyFile>>,
    pub commands: Option<Vec<String>>,
    pub environment_variables: Option<Vec<String>>,
    pub volumes: Option<Vec<VolumeMount>>,
    pub privileged: Option<bool>,
    pub shm_size: Option<i64>,
    pub user: Option<String>,
    pub skip_ready_check: Option<bool>,
    pub ztp_config: Option<String>,
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
    /// The keys in the `ssh_authorized_key_files` the client read, one OpenSSH line each.
    #[serde(default)]
    pub ssh_authorized_key_files_data: Option<Vec<String>>,
    pub kernel_cmdline: Option<String>,
    pub ready_port: Option<u16>,
    /// `[nodes.performance_tuning]`: CPU, memory, disk and interface tuning for a VM.
    pub performance_tuning: Option<PerformanceTuning>,
    /// A hardware watchdog for a VM, and what QEMU does when it expires. Absent, the VM has none.
    pub watchdog: Option<WatchdogAction>,
    /// The PTX platform a `juniper_cevolved` node emulates. Absent, it is BT.
    pub platform: Option<PtxPlatform>,
    /// Client-side provisioners, run by the CLI once the node is ready. Never serialized, so
    /// the client never sends them to the server and the server never stores them.
    #[serde(default, skip_serializing)]
    pub provisioners: Option<Vec<Provisioner>>,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct NodeExpanded {
    pub index: u16,
    pub name: String,
    pub model: NodeModel,
    pub image: Option<String>,
    pub version: Option<String>,
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
    pub boot_disk_size: Option<u16>,
    pub boot_disk_clone_method: Option<DiskImageCloneMethod>,
    pub data_interface_count: Option<u8>,
    pub ipv4_address: Option<Ipv4Addr>,
    pub ipv6_address: Option<Ipv6Addr>,
    pub text_files: Option<Vec<TextFileData>>,
    pub binary_files: Option<Vec<BinaryFileData>>,
    pub systemd_units: Option<Vec<SystemdUnitData>>,
    pub ssh_authorized_keys: Option<Vec<String>>,
    /// Keys the client read from the manifest's `ssh_authorized_key_files`.
    pub ssh_authorized_key_files: Option<Vec<String>>,
    pub commands: Option<Vec<String>>,
    pub environment_variables: Option<Vec<String>>,
    pub volumes: Option<Vec<VolumeMount>>,
    pub privileged: Option<bool>,
    pub shm_size: Option<i64>,
    pub user: Option<String>,
    pub skip_ready_check: Option<bool>,
    pub ztp_config: Option<String>,
    pub startup_scripts: Option<Vec<StartupScript>>,
    pub user_scripts: Option<Vec<StartupScript>>,
    pub kernel_cmdline: Option<String>,
    pub ready_port: Option<u16>,
    pub performance_tuning: Option<PerformanceTuning>,
    pub watchdog: Option<WatchdogAction>,
    pub platform: Option<PtxPlatform>,
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
            cpu_count: node.cpu_count,
            memory: node.memory,
            boot_disk_size: node.boot_disk_size,
            boot_disk_clone_method: node.boot_disk_clone_method,
            data_interface_count: node.data_interface_count,
            ipv4_address: None,
            ipv6_address: None,
            text_files: node.text_files_data.clone(),
            binary_files: node.binary_files_data.clone(),
            systemd_units: node.systemd_units_data.clone(),
            ssh_authorized_keys: node.ssh_authorized_keys.clone(),
            ssh_authorized_key_files: node.ssh_authorized_key_files_data.clone(),
            commands: node.commands.clone(),
            environment_variables: node.environment_variables.clone(),
            volumes: node.volumes.clone(),
            privileged: node.privileged,
            shm_size: node.shm_size,
            user: node.user.clone(),
            skip_ready_check: node.skip_ready_check,
            ztp_config: node.ztp_config.clone(),
            startup_scripts: node.startup_scripts_data.clone(),
            user_scripts: node.user_scripts_data.clone(),
            kernel_cmdline: node.kernel_cmdline.clone(),
            ready_port: node.ready_port,
            performance_tuning: node.performance_tuning.clone(),
            watchdog: node.watchdog,
            platform: node.platform,
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

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
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
            cpu_count: Some(4),
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
        assert_eq!(expanded.cpu_count, Some(4));
        assert_eq!(expanded.startup_scripts.unwrap()[0].content, "echo hi");
        assert_eq!(expanded.text_files.unwrap()[0].dst, "/etc/motd");
        assert!(expanded.ipv4_address.is_none());
    }
}
