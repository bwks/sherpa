use std::net::Ipv4Addr;

use serde_derive::{Deserialize, Serialize};

use shared::data::NodeModel;

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct Node {
    pub name: String,
    pub model: NodeModel,
    pub image: Option<String>,
    pub version: Option<String>,
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
    pub ipv4_address: Option<Ipv4Addr>,
    pub text_files: Option<Vec<TextFile>>,
    pub binary_files: Option<Vec<BinaryFile>>,
    pub systemd_units: Option<Vec<SystemdUnit>>,
    pub ssh_authorized_keys: Option<Vec<String>>,
    pub ssh_authorized_key_files: Option<Vec<AuthorizedKeyFile>>,
    pub commands: Option<Vec<String>>,
    pub environment_variables: Option<Vec<String>>,
    pub volumes: Option<Vec<VolumeMount>>,
    pub privileged: Option<bool>,
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
    pub ipv4_address: Option<Ipv4Addr>,
    pub text_files: Option<Vec<TextFile>>,
    pub binary_files: Option<Vec<BinaryFile>>,
    pub systemd_units: Option<Vec<SystemdUnit>>,
    pub ssh_authorized_keys: Option<Vec<String>>,
    pub ssh_authorized_key_files: Option<Vec<AuthorizedKeyFile>>,
    pub commands: Option<Vec<String>>,
    pub environment_variables: Option<Vec<String>>,
    pub volumes: Option<Vec<VolumeMount>>,
    pub privileged: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct VolumeMount {
    pub src: String,
    pub dst: String,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct TextFile {
    pub source: String,
    pub destination: String,
    pub user: String,
    pub group: String,
    pub permissions: u32,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct BinaryFile {
    pub source: String,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct SystemdUnit {
    pub name: String,
    pub source: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct AuthorizedKeyFile {
    pub source: String,
}
