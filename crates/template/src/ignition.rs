use std::net::{Ipv4Addr, Ipv6Addr};

use anyhow::{Context, Result};
use serde_derive::{Deserialize, Serialize};

use shared::data::{NetworkV4, NetworkV6};
use shared::konst::{IGNITION_VERSION, SHERPA_DOMAIN_NAME};
use shared::util::base64_encode;

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionConfig {
    pub ignition: Ignition,
    pub passwd: Passwd,
    pub storage: Storage,
    pub systemd: Systemd,
    pub networkd: Networkd,
    #[serde(rename = "kernelArguments", skip_serializing_if = "Option::is_none")]
    pub kernel_arguments: Option<KernelArguments>,
}

impl IgnitionConfig {
    #[must_use]
    pub fn new(
        users: Vec<IgnitionUser>,
        files: Vec<IgnitionFile>,
        links: Vec<IgnitionLink>,
        systemd_units: Vec<IgnitionUnit>,
        networkd_units: Vec<IgnitionUnit>,
        filesystems: Vec<IgnitionFileSystem>,
    ) -> IgnitionConfig {
        let directories = vec![IgnitionDirectory::default()];
        IgnitionConfig {
            ignition: Ignition::default(),
            passwd: Passwd { users },
            storage: Storage {
                files,
                links,
                directories,
                filesystems,
            },
            systemd: Systemd {
                units: systemd_units,
            },
            networkd: Networkd {
                units: networkd_units,
            },
            kernel_arguments: None,
        }
    }

    /// Serialize the `IgnitionConfig` to a pretty-printed JSON string.
    ///
    /// # Errors
    ///
    /// Fails when the configuration cannot be serialized.
    pub fn to_json_pretty(&self) -> Result<String> {
        serde_json::to_string_pretty(self).context("Failed to serialize the Ignition config")
    }
}

/// The kernel arguments Ignition makes the kernel command line have, which Flatcar adds to the
/// command line its bootloader builds, rebooting once to boot with them.
#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct KernelArguments {
    #[serde(rename = "shouldExist")]
    pub should_exist: Vec<String>,
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
            config: Config::default(),
            security: Security::default(),
            timeouts: Timeouts::default(),
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

#[derive(Serialize, Deserialize, Debug)]
pub struct Passwd {
    users: Vec<IgnitionUser>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionUser {
    pub name: String,
    #[serde(rename = "passwordHash", skip_serializing_if = "Option::is_none")]
    pub password_hash: Option<String>,
    #[serde(rename = "homeDir", skip_serializing_if = "Option::is_none")]
    pub home_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(rename = "sshAuthorizedKeys")]
    pub ssh_authorized_keys: Vec<String>,
    pub groups: Vec<String>,
}

/// A file's permission bits, which Ignition takes as a number: `0o644` is written as `420`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FileMode(u32);

impl FileMode {
    /// The mode with the permission bits `bits`, such as `0o644`.
    #[must_use]
    pub const fn new(bits: u32) -> Self {
        Self(bits)
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Storage {
    pub files: Vec<IgnitionFile>,
    pub links: Vec<IgnitionLink>,
    pub directories: Vec<IgnitionDirectory>,
    pub filesystems: Vec<IgnitionFileSystem>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionFileSystem {
    pub device: String,
    pub format: String,
    pub wipe_filesystem: bool,
    pub label: String,
}
impl Default for IgnitionFileSystem {
    fn default() -> Self {
        Self {
            device: "/dev/disk/by-label/data-disk".to_owned(),
            format: "ext4".to_owned(),
            wipe_filesystem: false,
            label: "data-disk".to_owned(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionDirectory {
    pub path: String,
    pub mode: FileMode,
    pub overwrite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<IgnitionFileParams>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<IgnitionFileParams>,
}

impl Default for IgnitionDirectory {
    fn default() -> Self {
        Self {
            path: "/opt/ztp".to_owned(),
            mode: FileMode::new(0o755),
            overwrite: false,
            user: None,
            group: None,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionFileParams {
    pub name: String,
}
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct IgnitionFile {
    pub path: String,
    pub mode: FileMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<IgnitionFileContents>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub append: Vec<IgnitionFileContents>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<IgnitionFileParams>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<IgnitionFileParams>,
}

impl IgnitionFile {
    /// Turn Flatcar's automatic updates off, so a lab node never reboots to update.
    #[must_use]
    pub fn disable_updates() -> Self {
        Self {
            path: "/etc/flatcar/update.conf".to_owned(),
            mode: FileMode::new(0o644),
            overwrite: Some(true),
            contents: Some(IgnitionFileContents::new("data:,REBOOT_STRATEGY%3Doff%0A")),
            ..Default::default()
        }
    }

    /// The hostname file of a node named `name`.
    #[must_use]
    pub fn hostname(name: &str) -> Self {
        Self {
            path: "/etc/hostname".to_owned(),
            mode: FileMode::new(0o644),
            contents: Some(IgnitionFileContents::new(&format!("data:,{name}"))),
            ..Default::default()
        }
    }

    /// A root-owned file at `path` with `mode` and `contents`, replacing any file there.
    #[must_use]
    pub fn root(path: &str, mode: u32, contents: &str) -> Self {
        Self {
            path: path.to_owned(),
            mode: FileMode::new(mode),
            overwrite: Some(true),
            contents: Some(IgnitionFileContents::base64(contents)),
            ..Default::default()
        }
    }

    /// The networkd configuration of the management interface, `eth0`.
    #[must_use]
    pub fn ztp_interface(
        mgmt_ipv4_address: Ipv4Addr,
        mgmt_ipv4: &NetworkV4,
        mgmt_ipv6_address: Option<Ipv6Addr>,
        mgmt_ipv6: Option<&NetworkV6>,
    ) -> Self {
        let ipv6 = match (mgmt_ipv6_address, mgmt_ipv6) {
            (Some(address), Some(v6)) => format!(
                "Address={address}/{}\nGateway={}\nDNS={}\n",
                v6.prefix_length, v6.first, v6.boot_server,
            ),
            _ => String::new(),
        };
        let contents = format!(
            r"[Match]
Name=eth0

[Network]
Address={address}/{prefix}
Gateway={gateway}
DNS={dns}
Domains={domain}
{ipv6}",
            address = mgmt_ipv4_address,
            prefix = mgmt_ipv4.prefix_length,
            gateway = mgmt_ipv4.first,
            dns = mgmt_ipv4.boot_server,
            domain = SHERPA_DOMAIN_NAME,
        );
        Self {
            path: "/etc/systemd/network/00-eth0.network".to_owned(),
            mode: FileMode::new(0o644),
            overwrite: Some(true),
            contents: Some(IgnitionFileContents::base64(&contents)),
            user: Some(IgnitionFileParams {
                name: "root".to_owned(),
            }),
            group: Some(IgnitionFileParams {
                name: "root".to_owned(),
            }),
            append: vec![],
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionLink {
    pub path: String,
    pub target: String,
    pub hard: bool,
    pub overwrite: bool,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct IgnitionFileContents {
    pub source: String,
    pub compression: Option<String>,
    pub verification: Verification,
}

impl IgnitionFileContents {
    #[must_use]
    pub fn new(source: &str) -> IgnitionFileContents {
        IgnitionFileContents {
            source: source.to_owned(),
            compression: None,
            verification: Verification::default(),
        }
    }

    /// The contents `contents`, carried as base64 in a data URL.
    #[must_use]
    pub fn base64(contents: &str) -> Self {
        Self::new(&format!("data:;base64,{}", base64_encode(contents)))
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Verification {}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct IgnitionUnit {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
}

impl IgnitionUnit {
    /// Mount the container data disk at `/media/container`.
    #[must_use]
    pub fn mount_container_disk() -> Self {
        Self {
            name: "media-container.mount".to_owned(),
            enabled: Some(true),
            contents: Some(
                r"[Unit]
Before=local-fs.target

[Mount]
What=/dev/disk/by-label/data-disk
Where=/media/container
Type=ext4

[Install]
WantedBy=local-fs.target
"
                .to_owned(),
            ),
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Systemd {
    units: Vec<IgnitionUnit>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Networkd {
    units: Vec<IgnitionUnit>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_config_042_ignition_an_append_only_file_has_no_contents() {
        let f = IgnitionFile {
            path: "/home/alice/.bash_profile".to_owned(),
            mode: FileMode::new(0o644),
            append: vec![IgnitionFileContents::new("data:,extra")],
            ..Default::default()
        };
        let json = serde_json::to_value(&f).unwrap();
        assert!(json.get("contents").is_none());
        assert_eq!(json["append"][0]["source"], "data:,extra");
    }

    #[test]
    fn generated_config_043_ignition_a_directory_carries_its_owner_and_group() {
        let dir = IgnitionDirectory {
            path: "/home/alice/.config".to_owned(),
            mode: FileMode::new(0o755),
            overwrite: false,
            user: Some(IgnitionFileParams {
                name: "alice".to_owned(),
            }),
            group: Some(IgnitionFileParams {
                name: "alice".to_owned(),
            }),
        };
        assert_eq!(
            serde_json::to_value(&dir).unwrap(),
            serde_json::json!({
                "path": "/home/alice/.config",
                "mode": 493,
                "overwrite": false,
                "user": { "name": "alice" },
                "group": { "name": "alice" },
            })
        );
    }
}
