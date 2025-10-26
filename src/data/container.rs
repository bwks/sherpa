use clap::ValueEnum;

use serde_derive::{Deserialize, Serialize};

use crate::core::konst::{
    CONTAINER_DHCP4_NAME, CONTAINER_DHCP4_REPO, CONTAINER_DHCP4_VERSION, CONTAINER_DNS_NAME,
    CONTAINER_DNS_REPO, CONTAINER_DNS_VERSION, CONTAINER_SRLINUX_NAME, CONTAINER_SRLINUX_REPO,
    CONTAINER_SRLINUX_VERSION, CONTAINER_TFTPD_NAME, CONTAINER_TFTPD_REPO, CONTAINER_TFTPD_VERSION,
    CONTAINER_WEBDIR_NAME, CONTAINER_WEBDIR_REPO, CONTAINER_WEBDIR_VERSION,
};

#[derive(PartialEq, Clone, Debug, Deserialize, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[clap(rename_all = "snake_case")]
pub enum ContainerModel {
    Tftpd,
    Webdir,
    Dns,
    Dhcp4,
    Srlinux,
}

#[derive(PartialEq, Clone, Debug, Deserialize, Serialize)]
pub struct ContainerImage {
    pub name: String,
    pub repo: String,
    pub version: String,
}

impl ContainerImage {
    pub fn tftpd() -> ContainerImage {
        ContainerImage {
            name: CONTAINER_TFTPD_NAME.to_owned(),
            repo: CONTAINER_TFTPD_REPO.to_owned(),
            version: CONTAINER_TFTPD_VERSION.to_owned(),
        }
    }
    pub fn webdir() -> ContainerImage {
        ContainerImage {
            name: CONTAINER_WEBDIR_NAME.to_owned(),
            repo: CONTAINER_WEBDIR_REPO.to_owned(),
            version: CONTAINER_WEBDIR_VERSION.to_owned(),
        }
    }
    pub fn dns() -> ContainerImage {
        ContainerImage {
            name: CONTAINER_DNS_NAME.to_owned(),
            repo: CONTAINER_DNS_REPO.to_owned(),
            version: CONTAINER_DNS_VERSION.to_owned(),
        }
    }
    pub fn dhcp4() -> ContainerImage {
        ContainerImage {
            name: CONTAINER_DHCP4_NAME.to_owned(),
            repo: CONTAINER_DHCP4_REPO.to_owned(),
            version: CONTAINER_DHCP4_VERSION.to_owned(),
        }
    }
    pub fn srlinux() -> ContainerImage {
        ContainerImage {
            name: CONTAINER_SRLINUX_NAME.to_owned(),
            repo: CONTAINER_SRLINUX_REPO.to_owned(),
            version: CONTAINER_SRLINUX_VERSION.to_owned(),
        }
    }
}
