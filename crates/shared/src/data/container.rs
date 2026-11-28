use serde_derive::{Deserialize, Serialize};

use crate::konst::{
    CONTAINER_DNSMASQ_NAME, CONTAINER_DNSMASQ_REPO, CONTAINER_DNSMASQ_VERSION,
    CONTAINER_WEBDIR_NAME, CONTAINER_WEBDIR_REPO, CONTAINER_WEBDIR_VERSION,
};

#[derive(PartialEq, Clone, Debug, Deserialize, Serialize)]
pub struct ContainerImage {
    pub name: String,
    pub repo: String,
    pub version: String,
}

impl ContainerImage {
    #[must_use]
    pub fn webdir() -> ContainerImage {
        ContainerImage {
            name: CONTAINER_WEBDIR_NAME.to_owned(),
            repo: CONTAINER_WEBDIR_REPO.to_owned(),
            version: CONTAINER_WEBDIR_VERSION.to_owned(),
        }
    }
    #[must_use]
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
    pub ipv6_address: Option<String>,
    pub linux_interface_name: Option<String>,
    pub admin_down: bool,
}

/// A file or folder on the server bind-mounted into a container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerMount {
    pub source: String,
    pub destination: String,
    pub read_only: bool,
}

impl ContainerMount {
    /// Mount `source` at `destination`, read-only.
    #[must_use]
    pub fn read_only(source: impl Into<String>, destination: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            read_only: true,
        }
    }

    /// Mount `source` at `destination`, writable.
    #[must_use]
    pub fn writable(source: impl Into<String>, destination: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            destination: destination.into(),
            read_only: false,
        }
    }

    /// Docker's bind form of the mount, `source:destination`, with `:ro` when read-only.
    #[must_use]
    pub fn bind(&self) -> String {
        let suffix = if self.read_only { ":ro" } else { "" };
        format!("{}:{}{suffix}", self.source, self.destination)
    }
}
