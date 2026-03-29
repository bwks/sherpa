use std::net::{Ipv4Addr, Ipv6Addr};

use serde_derive::{Deserialize, Serialize};

use shared::data::NodeModel;

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
    pub ipv4_address: Option<Ipv4Addr>,
    pub ipv6_address: Option<Ipv6Addr>,
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
    pub ipv4_address: Option<Ipv4Addr>,
    pub ipv6_address: Option<Ipv6Addr>,
    pub text_files: Option<Vec<TextFileData>>,
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
    pub startup_scripts: Option<Vec<StartupScript>>,
    pub user_scripts: Option<Vec<StartupScript>>,
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
