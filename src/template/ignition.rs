use anyhow::Result;

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
            device: "/dev/sdb".to_owned(),
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
pub struct File {
    // pub filesystem: String,
    pub path: String,
    pub mode: u16,
    pub contents: Contents,
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

#[derive(Serialize, Deserialize, Debug)]
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

#[derive(Serialize, Deserialize, Debug)]
pub struct Unit {
    pub name: String,
    pub enabled: bool,
    pub contents: String,
}

impl Unit {
    pub fn webdir() -> Self {
        Self {
            name: "webdir.service".to_owned(),
            enabled: true,
            contents: format!(r#"[Unit]
Description=WebDir
After=docker.service
Requires=docker.service

[Service]
TimeoutStartSec=0
ExecStartPre=/usr/bin/docker image pull ghcr.io/bwks/webdir:latest
ExecStart=/usr/bin/docker container run --rm --name webdir-app -p {HTTP_PORT}:{HTTP_PORT} -v /opt/ztp:/opt/ztp ghcr.io/bwks/webdir
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
TimeoutStartSec=0
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
    pub fn srlinux() -> Self {
        Self {
            name: "srlinux.service".to_owned(),
            enabled: true,
            contents: r#"[Unit]
Description=srlinux
After=docker.service media-container.mount
Requires=docker.service media-container.mount

[Service]
TimeoutStartSec=0
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
