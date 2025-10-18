use anyhow::Result;
use serde::Serializer;
use serde_derive::{Deserialize, Serialize};

use crate::core::konst::{HTTP_PORT, IGNITION_VERSION, TFTP_PORT};

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionConfig {
    pub ignition: Ignition,
    pub networkd: Networkd,
    pub passwd: Passwd,
    pub storage: Storage,
    pub systemd: Systemd,
}

impl IgnitionConfig {
    pub fn new(
        users: Vec<User>,
        files: Vec<File>,
        links: Vec<Link>,
        units: Vec<Unit>,
        filesystems: Vec<FileSystem>,
    ) -> IgnitionConfig {
        let directories = vec![Directory::default()];
        IgnitionConfig {
            ignition: Ignition::default(),
            networkd: Networkd::default(),
            passwd: Passwd { users },
            storage: Storage {
                files,
                links,
                directories,
                filesystems,
            },
            systemd: Systemd { units },
        }
    }
    /// Serialize the IgnitionConfig to a JSON string
    pub fn _to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| anyhow::anyhow!("JSON serialization error: {}", e))
    }

    /// Serialize the IgnitionConfig to a pretty-printed JSON string
    pub fn to_json_pretty(&self) -> Result<String> {
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
    #[serde(rename = "passwordHash")]
    pub password_hash: String,
    #[serde(rename = "sshAuthorizedKeys")]
    pub ssh_authorized_keys: Vec<String>,
    pub groups: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Storage {
    pub files: Vec<File>,
    pub links: Vec<Link>,
    pub directories: Vec<Directory>,
    pub filesystems: Vec<FileSystem>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FileSystem {
    pub device: String,
    pub format: String,
    pub wipe_filesystem: bool,
    pub label: String,
}
impl Default for FileSystem {
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
pub struct Directory {
    pub path: String,
    pub mode: u16,
    pub overwrite: bool,
}

impl Default for Directory {
    fn default() -> Self {
        Self {
            path: "/opt/ztp".to_owned(),
            mode: 755,
            overwrite: false,
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct FileParams {
    pub name: String,
}
#[derive(Serialize, Deserialize, Debug, Default)]
pub struct File {
    pub path: String,
    #[serde(serialize_with = "serialize_mode_as_decimal")]
    pub mode: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overwrite: Option<bool>,
    pub contents: Contents,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<FileParams>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<FileParams>,
}

impl File {
    pub fn disable_updates() -> Self {
        Self {
            path: "/etc/flatcar/update.conf".to_owned(),
            mode: 272,
            overwrite: Some(true),
            contents: Contents::new("data:,REBOOT_STRATEGY%3Doff%0A"),
            ..Default::default()
        }
    }
}

fn serialize_mode_as_decimal<S>(mode: &u32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Convert octal-like decimal directly to decimal
    let mode_str = mode.to_string();
    let decimal_mode = u32::from_str_radix(&mode_str, 8).unwrap_or(*mode); // fallback to original value if parsing fails

    serializer.serialize_u32(decimal_mode)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Link {
    pub path: String,
    pub target: String,
    pub hard: bool,
    pub overwrite: bool,
}
impl Default for Link {
    fn default() -> Self {
        Self {
            path: "/etc/systemd/system/multi-user.target.wants/docker.service".to_owned(),
            target: "/usr/lib/systemd/system/docker.service".to_owned(),
            hard: false,
            overwrite: true,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Contents {
    pub source: String,
    pub compression: Option<String>,
    pub verification: Verification,
}

impl Contents {
    pub fn new(source: &str) -> Contents {
        Contents {
            source: source.to_owned(),
            compression: None,
            verification: Verification::default(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Verification {}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Unit {
    pub name: String,
    pub enabled: bool,
    pub contents: String,
}

impl Unit {
    pub fn mount_container_disk() -> Self {
        Self {
            name: "media-container.mount".to_owned(),
            enabled: true,
            contents: r#"[Unit]
Before=local-fs.target

[Mount]
What=/dev/disk/by-label/data-disk
Where=/media/container
Type=ext4

[Install]
WantedBy=local-fs.target
"#
            .to_owned(),
        }
    }
    pub fn webdir() -> Self {
        Self {
            name: "webdir.service".to_owned(),
            enabled: true,
            contents: format!(r#"[Unit]
Description=WebDir
After=docker.service
Requires=docker.service

[Service]
TimeoutStartSec=infinity
ExecStartPre=/usr/bin/docker image pull ghcr.io/bwks/webdir:latest
ExecStart=/usr/bin/docker container run --rm --name webdir-app -p {HTTP_PORT}:{HTTP_PORT} -v /opt/ztp:/opt/ztp ghcr.io/bwks/westart ceos
ExecStop=/usr/bin/docker container stop webdir-app

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#).to_owned(),
        }
    }
    pub fn tftpd() -> Self {
        Self {
            name: "tftpd.service".to_owned(),
            enabled: true,
            contents: format!(r#"[Unit]
Description=TFTPd
After=docker.service
Requires=docker.service

[Service]
TimeoutStartSec=infinity
ExecStartPre=/usr/bin/docker image pull ghcr.io/bwks/tftpd:latest
ExecStart=/usr/bin/docker container run --rm --name tftpd-app -p {TFTP_PORT}:{TFTP_PORT}/udp -v /opt/ztp:/opt/ztp ghcr.io/bwks/tftpd
ExecStop=/usr/bin/docker container stop tftpd-app

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#).to_owned(),
        }
    }
    pub fn kubectl() -> Self {
        Self {
            name: "kubectl-install.service".to_owned(),
            enabled: true,
            contents: format!(
                r#"[Unit]
Description=Download and Install kubectl binary
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/bin/curl -L -o /opt/bin/kubectl https://dl.k8s.io/release/v1.33.2/bin/linux/amd64/kubectl
ExecStartPost=/usr/bin/chmod +x /opt/bin/kubectl

[Install]
WantedBy=multi-user.target
"#
            )
            .to_owned(),
        }
    }
    pub fn srlinux() -> Self {
        Self {
            name: "srlinux.service".to_owned(),
            enabled: true,
            contents: r#"[Unit]
Description=srlinux
After=media-container.mount docker.service
Requires=media-container.mount docker.service

[Service]
TimeoutStartSec=infinity
ExecStartPre=/usr/bin/docker load -i /media/container/image.tar.gz
ExecStart=sudo /usr/bin/docker container run --rm --privileged --name srlinux -p 2222:22/tcp ghcr.io/nokia/srlinux sudo bash /opt/srlinux/bin/sr_linux
ExecStop=/usr/bin/docker container stop srlinux

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#.to_owned(),
        }
    }
    pub fn ceos() -> Self {
        Self {
            name: "ceos.service".to_owned(),
            enabled: true,
            contents: r#"[Unit]
Description=ceos
After=media-container.mount docker.service
Requires=media-container.mount docker.service

[Service]
TimeoutStartSec=infinity
ExecStartPre=/usr/bin/docker image load -i /media/container/image.tar.gz
ExecStartPre=/usr/bin/docker container create --name ceos --privileged -p 2222:22/tcp -e INTFTYPE=eth -e ETBA=1 -e SKIP_ZEROTOUCH_BARRIER_IN_SYSDBINIT=1 -e CEOS=1 -e EOS_PLATFORM=ceoslab -e container=docker -e MAPETH0=1 -e MGMT_INTF=eth0 ceos:4.33.0f /sbin/init systemd.setenv=INTFTYPE=eth systemd.setenv=ETBA=1 systemd.setenv=SKIP_ZEROTOUCH_BARRIER_IN_SYSDBINIT=1 systemd.setenv=CEOS=1 systemd.setenv=EOS_PLATFORM=ceoslab systemd.setenv=container=docker systemd.setenv=MAPETH0=1 systemd.setenv=MGMT_INTF=eth0
ExecStart=/usr/bin/docker container start ceos
ExecStop=/usr/bin/docker container stop ceos

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#
            .to_owned(),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Systemd {
    units: Vec<Unit>,
}

impl Default for Systemd {
    fn default() -> Self {
        Self {
            units: vec![Unit::webdir(), Unit::tftpd()],
        }
    }
}
