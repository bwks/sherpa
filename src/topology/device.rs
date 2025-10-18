use serde_derive::{Deserialize, Serialize};

use crate::data::DeviceModels;

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct Device {
    pub name: String,
    pub model: DeviceModels,
    pub cpu_count: Option<u8>,
    pub memory: Option<u16>,
    pub text_files: Option<Vec<TextFile>>,
    pub binary_files: Option<Vec<BinaryFile>>,
    pub systemd_units: Option<Vec<SystemdUnit>>,
    pub ssh_authorized_keys: Option<Vec<String>>,
    pub ssh_authorized_key_files: Option<Vec<AuthorizedKeyFile>>,
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
