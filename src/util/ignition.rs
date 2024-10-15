use anyhow::Result;

use serde_derive::{Deserialize, Serialize};
use serde_json;

use crate::core::konst::IGNITION_VERSION;

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionConfig {
    pub ignition: Ignition,
    pub networkd: Networkd,
    pub passwd: Passwd,
    pub storage: Storage,
    pub systemd: Systemd,
}

impl IgnitionConfig {
    pub fn new(users: Vec<User>, files: Vec<File>) -> IgnitionConfig {
        IgnitionConfig {
            ignition: Ignition::default(),
            networkd: Networkd::default(),
            passwd: Passwd { users },
            storage: Storage { files },
            systemd: Systemd::default(),
        }
    }
    /// Serialize the IgnitionConfig to a JSON string
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| anyhow::anyhow!("JSON serialization error: {}", e))
    }

    /// Serialize the IgnitionConfig to a pretty-printed JSON string
    pub fn _to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("JSON serialization error: {}", e))
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Ignition {
    config: Config,
    security: Security,
    timeouts: Timeouts,
    version: String,
}
impl Default for Ignition {
    fn default() -> Self {
        Self {
            config: Default::default(),
            security: Default::default(),
            timeouts: Default::default(),
            version: IGNITION_VERSION.to_owned(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Config {}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Security {
    tls: Tls,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Tls {}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Timeouts {}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Networkd {}

#[derive(Serialize, Deserialize, Debug)]
pub struct Passwd {
    users: Vec<User>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct User {
    pub name: String,
    #[serde(rename = "sshAuthorizedKeys")]
    pub ssh_authorized_keys: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Storage {
    pub files: Vec<File>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct File {
    pub filesystem: String,
    pub path: String,
    pub contents: Contents,
    pub mode: u16,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Contents {
    pub source: String,
    pub verification: Verification,
}

impl Contents {
    pub fn new(source: &str) -> Contents {
        Contents {
            source: source.to_owned(),
            verification: Verification::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Verification {}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Systemd {}
