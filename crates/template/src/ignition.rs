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
    /// Omitted for an append-only file, so an existing file is kept and appended to.
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
        let encoded_contents = base64_encode(&contents);
        Self {
            path: "/etc/systemd/network/00-eth0.network".to_owned(),
            mode: FileMode::new(0o644),
            overwrite: Some(true),
            contents: Some(IgnitionFileContents::new(&format!(
                "data:;base64,{encoded_contents}"
            ))),
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
    use std::net::{Ipv4Addr, Ipv6Addr};

    use shared::data::{NetworkV4, NetworkV6};

    use super::*;

    // ── FileMode ──────────────────────────────────────────────────────────────

    #[test]
    fn test_file_mode_serializes_as_its_bits() {
        let file = IgnitionFile {
            path: "/etc/motd".to_owned(),
            mode: FileMode::new(0o644),
            ..Default::default()
        };
        assert!(
            serde_json::to_string(&file)
                .unwrap()
                .contains("\"mode\":420")
        );
    }

    // ── IgnitionFileSystem ────────────────────────────────────────────────────

    #[test]
    fn test_filesystem_default_values() {
        let fs = IgnitionFileSystem::default();
        assert_eq!(fs.device, "/dev/disk/by-label/data-disk");
        assert_eq!(fs.format, "ext4");
        assert!(!fs.wipe_filesystem);
        assert_eq!(fs.label, "data-disk");
    }

    // ── IgnitionDirectory ─────────────────────────────────────────────────────

    #[test]
    fn test_directory_default_values() {
        let d = IgnitionDirectory::default();
        assert_eq!(d.path, "/opt/ztp");
        assert_eq!(d.mode, FileMode::new(0o755));
        assert!(!d.overwrite);
    }

    // ── IgnitionFileContents ──────────────────────────────────────────────────

    #[test]
    fn test_file_contents_new_stores_source() {
        let source = "data:text/plain;base64,aGVsbG8=";
        let c = IgnitionFileContents::new(source);
        assert_eq!(c.source, source);
    }

    // ── IgnitionFile factory functions ────────────────────────────────────────

    #[test]
    fn test_disable_updates_path_and_mode() {
        let f = IgnitionFile::disable_updates();
        assert_eq!(f.path, "/etc/flatcar/update.conf");
        assert_eq!(f.mode, FileMode::new(0o644));
        assert_eq!(f.overwrite, Some(true));
    }

    fn make_netv4() -> NetworkV4 {
        let prefix: ipnet::Ipv4Net = "10.0.0.0/24".parse().unwrap();
        NetworkV4 {
            prefix,
            prefix_length: 24,
            first: "10.0.0.1".parse().unwrap(),
            last: "10.0.0.254".parse().unwrap(),
            boot_server: "10.0.0.1".parse().unwrap(),
            network: "10.0.0.0".parse().unwrap(),
            subnet_mask: "255.255.255.0".parse().unwrap(),
            hostmask: "0.0.0.255".parse().unwrap(),
        }
    }

    fn make_netv6() -> NetworkV6 {
        let prefix: ipnet::Ipv6Net = "2001:db8::/64".parse().unwrap();
        NetworkV6 {
            prefix,
            prefix_length: 64,
            first: "2001:db8::1".parse().unwrap(),
            last: "2001:db8::fffe".parse().unwrap(),
            boot_server: "2001:db8::1".parse().unwrap(),
            network: "2001:db8::".parse().unwrap(),
        }
    }

    #[test]
    fn test_ztp_interface_ipv4_only() {
        let addr: Ipv4Addr = "10.0.0.5".parse().unwrap();
        let f = IgnitionFile::ztp_interface(addr, &make_netv4(), None, None);
        assert_eq!(f.path, "/etc/systemd/network/00-eth0.network");
        assert_eq!(f.mode, FileMode::new(0o644));
        assert_eq!(f.user.as_ref().unwrap().name, "root");
        assert_eq!(f.group.as_ref().unwrap().name, "root");
        assert!(
            f.contents
                .as_ref()
                .unwrap()
                .source
                .starts_with("data:;base64,")
        );
    }

    #[test]
    fn test_ztp_interface_dual_stack() {
        let addr: Ipv4Addr = "10.0.0.5".parse().unwrap();
        let v6_addr: Ipv6Addr = "2001:db8::5".parse().unwrap();
        let v6_net = make_netv6();
        let f = IgnitionFile::ztp_interface(addr, &make_netv4(), Some(v6_addr), Some(&v6_net));
        assert_eq!(f.path, "/etc/systemd/network/00-eth0.network");
        assert!(f.contents.as_ref().unwrap().source.len() > "data:;base64,".len());
    }

    // ── IgnitionUnit factory functions ────────────────────────────────────────

    #[test]
    fn test_unit_mount_container_disk() {
        let u = IgnitionUnit::mount_container_disk();
        assert_eq!(u.name, "media-container.mount");
        assert_eq!(u.enabled, Some(true));
    }

    // ── IgnitionConfig ────────────────────────────────────────────────────────

    #[test]
    fn test_ignition_config_new_serializes_to_json() {
        let config = IgnitionConfig::new(vec![], vec![], vec![], vec![], vec![], vec![]);
        let json = config.to_json_pretty().unwrap();
        assert!(json.contains("ignition"));
        assert!(json.contains("version"));
    }

    // ── directories ───────────────────────────────────────────────────────────

    #[test]
    fn test_directory_mode_serialized_as_decimal() {
        let json = serde_json::to_value(IgnitionDirectory::default()).unwrap();
        assert_eq!(json["mode"], 0o755);
        assert!(json.get("user").is_none());
    }

    #[test]
    fn test_directory_owner_serialized() {
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
        let json = serde_json::to_value(&dir).unwrap();
        assert_eq!(json["user"]["name"], "alice");
        assert_eq!(json["group"]["name"], "alice");
    }

    // ── append ────────────────────────────────────────────────────────────────

    #[test]
    fn test_append_only_file_omits_contents() {
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
    fn test_file_without_append_omits_append() {
        let json = serde_json::to_value(IgnitionFile::disable_updates()).unwrap();
        assert!(json.get("append").is_none());
        assert!(json.get("contents").is_some());
    }

    #[test]
    fn test_file_mode_serialized_as_decimal() {
        // 0o644 is 420 in decimal.
        let json = serde_json::to_value(IgnitionFile::disable_updates()).unwrap();
        assert_eq!(json["mode"], 420);
    }
}
