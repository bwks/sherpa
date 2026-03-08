use clap::ValueEnum;

use serde_derive::{Deserialize, Serialize};

use crate::konst::{
    CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO, CONTAINER_DNSMASQ_VERSION,
    CONTAINER_WEBDIR_NAME, CONTAINER_WEBDIR_REPO, CONTAINER_WEBDIR_VERSION,
};

#[derive(PartialEq, Clone, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum ContainerModel {
    Webdir,
    Dnsmasq,
    Srlinux,
}

#[derive(PartialEq, Clone, Debug, Deserialize, Serialize)]
pub struct ContainerImage {
    pub name: String,
    pub repo: String,
    pub version: String,
}

impl ContainerImage {
    pub fn webdir() -> ContainerImage {
        ContainerImage {
            name: CONTAINER_WEBDIR_NAME.to_owned(),
            repo: CONTAINER_WEBDIR_REPO.to_owned(),
            version: CONTAINER_WEBDIR_VERSION.to_owned(),
        }
    }
    pub fn dnsmasq() -> ContainerImage {
        ContainerImage {
            name: CONTAINER_DNSMASQ_NAME.to_owned(),
            repo: CONTAINER_DNSMASQ_REPO.to_owned(),
            version: CONTAINER_DNSMASQ_VERSION.to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ContainerNetworkAttachment {
    pub name: String,
    pub ipv4_address: Option<String>,
    /// When set, the Docker-assigned interface will be renamed to this name
    /// inside the container after network attachment (e.g. `e1-1` for SR Linux).
    pub linux_interface_name: Option<String>,
    /// When `true`, the interface is renamed but left admin-down (no promisc, no `ip link set up`).
    /// Used for disabled/unused interfaces attached to the isolated bridge.
    pub admin_down: bool,
}
