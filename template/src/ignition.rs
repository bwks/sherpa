use anyhow::Result;
use serde::Serializer;
use serde_derive::{Deserialize, Serialize};
use serde_json;

use konst::{DOCKER_COMPOSE_VERSION, IGNITION_VERSION};

#[derive(Serialize, Deserialize, Debug)]
pub struct IgnitionConfig {
    pub ignition: Ignition,
    pub passwd: Passwd,
    pub storage: Storage,
    pub systemd: Systemd,
    pub networkd: Networkd,
}

impl IgnitionConfig {
    pub fn new(
        users: Vec<User>,
        files: Vec<File>,
        links: Vec<Link>,
        systemd_units: Vec<Unit>,
        networkd_units: Vec<Unit>,
        filesystems: Vec<FileSystem>,
    ) -> IgnitionConfig {
        let directories = vec![Directory::default()];
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
    pub fn disable_resolved() -> Self {
        Self {
            path: "/etc/systemd/resolved.conf.d/no-stub.conf".to_owned(),
            mode: 644,
            overwrite: Some(true),
            contents: Contents::new("data:text/plain;base64,RE5TU3R1Ykxpc3RlbmVyPW5vCg=="),
            ..Default::default()
        }
    }

    pub fn disable_updates() -> Self {
        Self {
            path: "/etc/flatcar/update.conf".to_owned(),
            mode: 644,
            overwrite: Some(true),
            contents: Contents::new("data:,REBOOT_STRATEGY%3Doff%0A"),
            ..Default::default()
        }
    }
    pub fn docker_compose_raw() -> Self {
        Self {
            path: format!(
                "/opt/extensions/docker-compose/docker-compose-{DOCKER_COMPOSE_VERSION}-x86-64.raw"
            ),
            mode: 644,
            overwrite: Some(true),
            contents: Contents::new(&format!(
                "https://extensions.flatcar.org/extensions/docker-compose-{DOCKER_COMPOSE_VERSION}-x86-64.raw"
            )),
            ..Default::default()
        }
    }
    pub fn docker_compose_conf() -> Self {
        Self {
            path: "/etc/sysupdate.docker-compose.d/docker-compose.conf".to_owned(),
            mode: 644,
            overwrite: Some(true),
            contents: Contents::new(
                "https://extensions.flatcar.org/extensions/docker-compose.conf",
            ),
            ..Default::default()
        }
    }
    pub fn systemd_noop() -> Self {
        Self {
            path: "/etc/sysupdate.d/noop.conf".to_owned(),
            mode: 644,
            overwrite: Some(true),
            contents: Contents::new("https://extensions.flatcar.org/extensions/noop.conf"),
            ..Default::default()
        }
    }
    //     pub fn ztp_interface(config: &SherpaConfig) -> Result<Self> {
    //         let mgmt_net = config.management_prefix_ipv4;
    //         let contents = format!(
    //             r#"[Match]
    // Name=eth0

    // [Network]
    // Address={address}/{prefix}
    // Gateway={gateway}
    // DNS={gateway}
    // Domains={domain}
    // "#,
    //             address = mgmt_net.nth(SHERPA_MANAGEMENT_DNSMASQ_IPV4_INDEX).unwrap(),
    //             prefix = mgmt_net.prefix(),
    //             gateway = mgmt_net.nth(1).unwrap(),
    //             domain = SHERPA_DOMAIN_NAME,
    //         );
    //         let encoded_contents = base64_encode(&contents);
    //         Ok(Self {
    //             path: "/etc/systemd/network/00-eth0.network".to_owned(),
    //             mode: 644,
    //             overwrite: Some(true),
    //             contents: Contents::new(&format!("data:;base64,{encoded_contents}")),
    //             // contents: Contents::new(&contents),
    //             user: Some(FileParams {
    //                 name: "root".to_owned(),
    //             }),
    //             group: Some(FileParams {
    //                 name: "root".to_owned(),
    //             }),
    //         })
    //     }
    pub fn dnsmasq_config(config: &str) -> Self {
        Self {
            path: "/opt/dnsmasq/dnsmasq.conf".to_owned(),
            mode: 644,
            overwrite: Some(true),
            contents: Contents::new(&format!("data:text/plain;base64,{config}")),
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Link {
    pub path: String,
    pub target: String,
    pub hard: bool,
    pub overwrite: bool,
}
impl Link {
    pub fn docker_compose_raw() -> Self {
        Self {
            path: "/etc/extensions/docker-compose.raw".to_owned(),
            target: format!(
                "/opt/extensions/docker-compose/docker-compose-{DOCKER_COMPOSE_VERSION}-x86-64.raw"
            ),
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

#[derive(Serialize, Clone, Deserialize, Debug, Default)]
pub struct Dropin {
    name: String,
    contents: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Unit {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mask: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropins: Option<Vec<Dropin>>,
}

impl Unit {
    pub fn systemd_resolved() -> Self {
        Self {
            name: "systemd-resolved.service".to_owned(),
            enabled: Some(false),
            mask: Some(true),
            ..Default::default()
        }
    }
    pub fn systemd_update_timer() -> Self {
        Self {
            name: "systemd-sysupdate.timer".to_owned(),
            enabled: Some(true),
            ..Default::default()
        }
    }
    pub fn systemd_update_service() -> Self {
        Self {
            name: "systemd-sysupdate.service".to_owned(),
            dropins: Some(vec![
                Dropin {
                    name: "docker-compose.conf".to_owned(),
                    contents: r#"[Service]
ExecStartPre=/usr/bin/sh -c "readlink --canonicalize /etc/extensions/docker-compose.raw > /tmp/docker-compose"
ExecStartPre=/usr/lib/systemd/systemd-sysupdate -C docker-compose update
ExecStartPost=/usr/bin/sh -c "readlink --canonicalize /etc/extensions/docker-compose.raw > /tmp/docker-compose-new"
ExecStartPost=/usr/bin/sh -c "if ! cmp --silent /tmp/docker-compose /tmp/docker-compose-new; then touch /run/reboot-required; fi"
"#.to_owned(),
                }
            ]),
            ..Default::default()
        }
    }
    pub fn mount_container_disk() -> Self {
        Self {
            name: "media-container.mount".to_owned(),
            enabled: Some(true),
            contents: Some(
                r#"[Unit]
Before=local-fs.target

[Mount]
What=/dev/disk/by-label/data-disk
Where=/media/container
Type=ext4

[Install]
WantedBy=local-fs.target
"#
                .to_owned(),
            ),
            ..Default::default()
        }
    }
    pub fn dnsmasq() -> Self {
        Self {
            name: "dnsmasq.service".to_owned(),
            enabled: Some(true),
            contents: Some(
                format!(
                    r#"[Unit]
Description=dnsmasq
After=media-container.mount containerd.service
Requires=media-container.mount containerd.service

[Service]
TimeoutStartSec=infinity
ExecStartPre=/usr/bin/mkdir -p /opt/ztp/dnsmasq
ExecStartPre=/usr/bin/mkdir -p /opt/ztp/images
ExecStartPre=/usr/bin/touch /opt/ztp/dnsmasq/leases.txt
ExecStartPre=/usr/bin/bash -c 'chmod -R a+r /opt/ztp/'
ExecStartPre=/usr/bin/docker load -i /media/container/dnsmasq.tar.gz
ExecStart=/usr/bin/docker container run --rm --name dnsmasq-app --network host -v /opt/dnsmasq/dnsmasq.conf:/etc/dnsmasq.conf -v /opt/ztp/dnsmasq/leases.txt:/var/lib/misc/dnsmasq.leases -v /opt/ztp/tftp:/opt/ztp/tftp --cap-add=NET_ADMIN dockurr/dnsmasq
ExecStop=/usr/bin/docker container stop dnsmasq-app

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#
                )
                .to_owned(),
            ),
            ..Default::default()
        }
    }
    pub fn webdir() -> Self {
        Self {
            name: "webdir.service".to_owned(),
            enabled: Some(true),
            contents: Some(format!(r#"[Unit]
Description=WebDir
After=media-container.mount containerd.service
Requires=media-container.mount containerd.service

[Service]
TimeoutStartSec=infinity
ExecStartPre=/usr/bin/mkdir -p /opt/ztp/configs
ExecStartPre=/usr/bin/bash -c 'chmod -R a+r /opt/ztp/'
ExecStartPre=/usr/bin/docker load -i /media/container/webdir.tar.gz
ExecStart=/usr/bin/docker container run --rm --name webdir-app --network host -v /opt/ztp:/opt/ztp:ro ghcr.io/bwks/webdir
ExecStop=/usr/bin/docker container stop webdir-app

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#).to_owned()),
            ..Default::default()
        }
    }
    pub fn srlinux() -> Self {
        Self {
            name: "srlinux.service".to_owned(),
            enabled: Some(true),
            contents: Some(r#"[Unit]
Description=srlinux
After=media-container.mount containerd.service
Requires=media-container.mount containerd.service

[Service]
TimeoutStartSec=infinity
ExecStartPre=/usr/bin/docker load -i /media/container/image.tar.gz
ExecStart=sudo /usr/bin/docker container run --rm --privileged --name srlinux -p 2222:22/tcp ghcr.io/nokia/srlinux sudo bash /opt/srlinux/bin/sr_linux
ExecStop=/usr/bin/docker container stop srlinux

Restart=always
RestartSec=5s

[Install]
WantedBy=multi-user.target
"#.to_owned()),
            ..Default::default()
        }
    }
    pub fn ceos() -> Self {
        Self {
            name: "ceos.service".to_owned(),
            enabled: Some(true),
            contents: Some(r#"[Unit]
Description=ceos
After=media-container.mount containerd.service
Requires=media-container.mount containerd.service

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
            .to_owned()),
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Systemd {
    units: Vec<Unit>,
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Networkd {
    units: Vec<Unit>,
}

/// Convert a unix octal permission mode (base 8) to it'd decimal equivalent (base 10).
/// EG: 644 -> 420
fn serialize_mode_as_decimal<S>(mode: &u32, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    // Convert octal-like decimal directly to decimal
    let mode_str = mode.to_string();
    let decimal_mode = u32::from_str_radix(&mode_str, 8).unwrap_or(*mode); // fallback to original value if parsing fails

    serializer.serialize_u32(decimal_mode)
}
