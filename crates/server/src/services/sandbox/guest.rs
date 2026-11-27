//! What a sandbox's guest is given: its interfaces and management addresses, and the config
//! drive with its Ignition config, the body Docker creates its container from, and its
//! container's volumes.

use std::fs;
use std::io::ErrorKind;
use std::iter;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::Path;

use anyhow::{Context, Result, bail};

use container::ContainerSpec;
use shared::data::{
    ConnectionTypes, ContainerMount, Dns, Interface, ModelKind, NodeModel, SherpaNetwork,
};
use shared::konst::{KVM_OUI, MTU_JUMBO_NET, MTU_STD, SHERPA_STORAGE_POOL_PATH};
use shared::util;
use template::{
    GuestAddress, GuestInterface, SANDBOX_CONFIG_DRIVE_LABEL, SANDBOX_CONFIG_DRIVE_MOUNT,
    SANDBOX_CONFIG_DRIVE_USER_DATA, SANDBOX_CONTAINER_BODY, SANDBOX_MANAGEMENT_INTERFACE,
    WritableVolume,
};

use crate::konst::{NODE_CONFIG_DIR_MODE, NODE_CONFIG_FILE_MODE};
use crate::services::p2p_links::ContainerVeth;

/// The folder on the config drive that holds the container's volumes, one per mount.
const DRIVE_VOLUMES_DIR: &str = "sherpa/volumes";
/// The ISO 9660 level of a volume on the config drive, counting the drive's root as 1.
pub(crate) const DRIVE_VOLUME_DEPTH: usize = 4;

/// The interfaces of a sandbox: its NICs, the management NIC first, the data interfaces on no
/// link, which the guest makes, and the domain's NICs, the management NIC first, each with the
/// MAC the guest finds it by.
pub(super) struct GuestNics {
    pub(super) interfaces: Vec<GuestInterface>,
    pub(super) placeholders: Vec<GuestInterface>,
    pub(super) domain: Vec<Interface>,
}

/// A volume of the container on the config drive: its source on the server and its folder or
/// file on the drive.
pub(super) struct DriveVolume {
    source: String,
    on_drive: String,
}

/// The container's read-only volumes as the guest mounts them, what goes on the config drive,
/// and the writable volumes the guest copies off it into the container.
pub(super) struct GuestVolumes {
    pub(super) mounts: Vec<ContainerMount>,
    pub(super) drive: Vec<DriveVolume>,
    pub(super) writable: Vec<WritableVolume>,
}

/// The interfaces of a sandbox of `model_kind`, each with a MAC of its own: the management NIC,
/// a NIC for each data interface on a link or bridge, and each data interface on no link, each
/// named as the model expects.
///
/// # Errors
///
/// Fails when an interface index has no name for the model.
pub(super) fn guest_nics(model_kind: ModelKind, veths: &[ContainerVeth]) -> Result<GuestNics> {
    let management_mac = util::random_mac(KVM_OUI);
    let mut guest = GuestNics {
        interfaces: vec![GuestInterface {
            mac: management_mac.clone(),
            name: SANDBOX_MANAGEMENT_INTERFACE.to_owned(),
        }],
        placeholders: Vec::new(),
        domain: vec![Interface {
            name: SANDBOX_MANAGEMENT_INTERFACE.to_owned(),
            num: 0,
            mac_address: management_mac,
            mtu: MTU_STD,
            connection_type: ConnectionTypes::Management,
            tap: None,
        }],
    };
    for veth in veths {
        let interface = GuestInterface {
            mac: util::random_mac(KVM_OUI),
            name: util::container_interface_name(model_kind, veth.interface_idx)?,
        };
        if veth.admin_down {
            guest.placeholders.push(interface);
            continue;
        }
        let (name, connection_type, tap) = match &veth.bridge {
            Some(bridge) => (
                bridge.clone(),
                ConnectionTypes::PrivateBridge,
                Some(veth.host_veth.clone()),
            ),
            None => (veth.host_veth.clone(), ConnectionTypes::P2p, None),
        };
        guest.domain.push(Interface {
            name,
            num: veth.interface_idx,
            mac_address: interface.mac.clone(),
            mtu: MTU_JUMBO_NET,
            connection_type,
            tap,
        });
        guest.interfaces.push(interface);
    }
    Ok(guest)
}

/// The addresses of a sandbox's management NIC: the node's IPv4 address, and its IPv6 address
/// when the lab's management network has IPv6, each with the network's prefix length and its
/// gateway.
pub(super) fn management_addresses(
    mgmt_net: &SherpaNetwork,
    ipv4: Ipv4Addr,
    ipv6: Option<Ipv6Addr>,
) -> Vec<GuestAddress> {
    let mut addresses = vec![GuestAddress {
        address: IpAddr::V4(ipv4),
        prefix_length: mgmt_net.v4.prefix_length,
        gateway: IpAddr::V4(mgmt_net.v4.first),
    }];
    if let Some((address, v6)) = ipv6.zip(mgmt_net.v6.as_ref()) {
        addresses.push(GuestAddress {
            address: IpAddr::V6(address),
            prefix_length: v6.prefix_length,
            gateway: IpAddr::V6(v6.first),
        });
    }
    addresses
}

/// The command the VM's serial console runs in the node's container: its network OS CLI.
pub(super) fn console_command(model_kind: ModelKind) -> &'static [&'static str] {
    match model_kind.model() {
        NodeModel::AristaEos => &["Cli"],
        NodeModel::NokiaSrlinux => &["sr_cli"],
        _ => &["bash"],
    }
}

/// The container's `volumes` as the guest has them: each on the config drive as
/// `sherpa/volumes/N`, a read-only one mounted from the drive, and a writable one copied into the
/// container when it is created.
pub(super) fn guest_volumes(volumes: &[ContainerMount]) -> GuestVolumes {
    let mut guest = GuestVolumes {
        mounts: Vec::new(),
        drive: Vec::new(),
        writable: Vec::new(),
    };
    for (index, volume) in volumes.iter().enumerate() {
        let on_drive = format!("{DRIVE_VOLUMES_DIR}/{index}");
        let from_drive = format!("{SANDBOX_CONFIG_DRIVE_MOUNT}/{on_drive}");
        if volume.read_only {
            guest.mounts.push(ContainerMount {
                source: from_drive,
                ..volume.clone()
            });
        } else {
            guest.writable.push(WritableVolume {
                source: from_drive,
                destination: volume.destination.clone(),
            });
        }
        guest.drive.push(DriveVolume {
            source: volume.source.clone(),
            on_drive,
        });
    }
    guest
}

/// The JSON body Docker in the guest creates the container `spec` from, with its volumes
/// mounted as `mounts` and the lab's `dns`.
///
/// # Errors
///
/// Fails when the spec's resource limits do not fit Docker's or the body cannot be serialized.
pub(super) fn guest_container_body(
    spec: &ContainerSpec,
    mounts: Vec<ContainerMount>,
    dns: &Dns,
) -> Result<String> {
    let body = container::sandbox_create_body(
        ContainerSpec {
            volumes: mounts,
            ..spec.clone()
        },
        dns,
    )
    .with_context(|| format!("Invalid container {}", spec.name))?;
    serde_json::to_string(&body)
        .with_context(|| format!("Cannot serialize the body of container {}", spec.name))
}

/// Write config drive `drive` in the storage pool with Ignition config `user_data`, the body
/// `container_body` Docker creates the node's container from, when the VM runs one, and
/// `volumes`, staged in `stage`, and return its path.
///
/// # Errors
///
/// Fails when a volume's source cannot be copied or the drive cannot be written.
pub(super) fn write_config_drive(
    stage: &Path,
    drive: &str,
    user_data: &str,
    container_body: Option<&str>,
    volumes: &[DriveVolume],
) -> Result<String> {
    remove_stage(stage)?;
    let written = stage_config_drive(stage, user_data, container_body, volumes).and_then(|()| {
        let path = format!("{SHERPA_STORAGE_POOL_PATH}/{drive}");
        iso::create_iso(stage, &path, SANDBOX_CONFIG_DRIVE_LABEL)
            .with_context(|| format!("Cannot write config drive {path}"))?;
        Ok(path)
    });
    remove_stage(stage)?;
    written
}

/// Remove staging folder `stage` when it is there.
fn remove_stage(stage: &Path) -> Result<()> {
    match fs::remove_dir_all(stage) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(error).with_context(|| format!("Cannot remove {}", stage.display()))
        }
        _ => Ok(()),
    }
}

/// Stage the config drive's files in `stage`: the Ignition config, the container's body when
/// there is one, and each volume's file or folder.
fn stage_config_drive(
    stage: &Path,
    user_data: &str,
    container_body: Option<&str>,
    volumes: &[DriveVolume],
) -> Result<()> {
    let files = iter::once((SANDBOX_CONFIG_DRIVE_USER_DATA, user_data))
        .chain(container_body.map(|body| (SANDBOX_CONTAINER_BODY, body)));
    for (on_drive, contents) in files {
        let path = stage.join(on_drive);
        create_parent(&path)?;
        util::write_file_with_permissions(&path, contents.as_bytes(), NODE_CONFIG_FILE_MODE)?;
    }
    for volume in volumes {
        let destination = stage.join(&volume.on_drive);
        create_parent(&destination)?;
        copy_volume(Path::new(&volume.source), &destination)?;
    }
    Ok(())
}

/// Create the folders above `path`.
fn create_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no folder", path.display()))?;
    util::create_dir_with_permissions(parent, NODE_CONFIG_DIR_MODE)
}

/// Copy file or folder `source` to `destination`, a folder's entries recursively.
///
/// # Errors
///
/// Fails when `source` cannot be read, or is neither a file nor a folder, or a copy cannot be
/// written.
fn copy_volume(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Cannot read volume source {}", source.display()))?;
    if metadata.is_file() {
        fs::copy(source, destination).with_context(|| {
            format!(
                "Cannot copy volume source {} to {}",
                source.display(),
                destination.display()
            )
        })?;
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!(
            "Volume source {} is neither a file nor a folder",
            source.display()
        );
    }
    util::create_dir_with_permissions(destination, NODE_CONFIG_DIR_MODE)?;
    for entry in fs::read_dir(source)
        .with_context(|| format!("Cannot read volume folder {}", source.display()))?
    {
        let entry = entry?;
        copy_volume(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ipnet::{Ipv4Net, Ipv6Net};
    use tempfile::TempDir;

    use shared::data::{NetworkV4, NetworkV6, NodeKind};

    use super::*;

    /// A lab's management network, `172.31.0.0/24` with IPv6 `fd00:b0d1:0:1::/64` when `v6`.
    fn mgmt_net(v6: bool) -> SherpaNetwork {
        let v4_prefix: Ipv4Net = "172.31.0.0/24".parse().unwrap();
        let v6_prefix: Ipv6Net = "fd00:b0d1:0:1::/64".parse().unwrap();
        SherpaNetwork {
            v4: NetworkV4 {
                prefix: v4_prefix,
                first: Ipv4Addr::new(172, 31, 0, 1),
                last: v4_prefix.broadcast(),
                boot_server: Ipv4Addr::new(172, 31, 0, 2),
                network: v4_prefix.network(),
                subnet_mask: v4_prefix.netmask(),
                hostmask: v4_prefix.hostmask(),
                prefix_length: v4_prefix.prefix_len(),
            },
            v6: v6.then(|| NetworkV6 {
                prefix: v6_prefix,
                first: "fd00:b0d1:0:1::1".parse().unwrap(),
                last: v6_prefix.broadcast(),
                boot_server: "fd00:b0d1:0:1::2".parse().unwrap(),
                network: v6_prefix.network(),
                prefix_length: v6_prefix.prefix_len(),
            }),
        }
    }

    #[test]
    fn sandbox_103_each_wired_data_interface_is_a_domain_nic_with_the_mac_the_guest_names_it_by() {
        let model_kind = ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container)).unwrap();
        let veth = |host_veth: &str, interface_idx: u8, admin_down: bool, bridge: Option<&str>| {
            ContainerVeth {
                host_veth: host_veth.to_owned(),
                host_alias: String::new(),
                interface_idx,
                model_kind,
                admin_down,
                bridge: bridge.map(str::to_owned),
            }
        };
        let nics = guest_nics(
            model_kind,
            &[
                veth("tpa1-lab1", 1, false, None),
                veth("cb3i2-lab1", 2, false, Some("brs0-lab1")),
                veth("cd3i3-lab1", 3, true, None),
            ],
        )
        .unwrap();
        let domain: Vec<(&str, &ConnectionTypes, Option<&str>, u16)> = nics
            .domain
            .iter()
            .map(|nic| {
                (
                    nic.name.as_str(),
                    &nic.connection_type,
                    nic.tap.as_deref(),
                    nic.mtu,
                )
            })
            .collect();
        assert_eq!(
            domain,
            [
                ("eth0", &ConnectionTypes::Management, None, MTU_STD),
                ("tpa1-lab1", &ConnectionTypes::P2p, None, MTU_JUMBO_NET),
                (
                    "brs0-lab1",
                    &ConnectionTypes::PrivateBridge,
                    Some("cb3i2-lab1"),
                    MTU_JUMBO_NET
                ),
            ]
        );
        let domain_macs: Vec<&str> = nics
            .domain
            .iter()
            .map(|nic| nic.mac_address.as_str())
            .collect();
        let guest_macs: Vec<&str> = nics
            .interfaces
            .iter()
            .map(|interface| interface.mac.as_str())
            .collect();
        assert_eq!(domain_macs, guest_macs);
        assert_eq!(nics.placeholders.len(), 1);
        assert!(!domain_macs.contains(&nics.placeholders[0].mac.as_str()));
    }

    #[test]
    fn sandbox_045_the_management_nic_gets_the_nodes_addresses_with_the_labs_gateways() {
        let ipv4 = Ipv4Addr::new(172, 31, 0, 23);
        let ipv6: Ipv6Addr = "fd00:b0d1:0:1::17".parse().unwrap();
        let shown = |addresses: Vec<GuestAddress>| -> Vec<String> {
            addresses
                .iter()
                .map(|a| format!("{}/{} {}", a.address, a.prefix_length, a.gateway))
                .collect()
        };
        assert_eq!(
            shown(management_addresses(&mgmt_net(true), ipv4, Some(ipv6))),
            [
                "172.31.0.23/24 172.31.0.1",
                "fd00:b0d1:0:1::17/64 fd00:b0d1:0:1::1"
            ]
        );
        assert_eq!(
            shown(management_addresses(&mgmt_net(true), ipv4, None)),
            ["172.31.0.23/24 172.31.0.1"]
        );
        assert_eq!(
            shown(management_addresses(&mgmt_net(false), ipv4, Some(ipv6))),
            ["172.31.0.23/24 172.31.0.1"]
        );
    }

    #[test]
    fn sandbox_013_the_console_runs_each_models_cli() {
        for (model, cli) in [
            (NodeModel::AristaEos, "Cli"),
            (NodeModel::NokiaSrlinux, "sr_cli"),
            (NodeModel::SonicLinux, "bash"),
        ] {
            let model_kind = ModelKind::new(model, Some(NodeKind::Container)).unwrap();
            assert_eq!(console_command(model_kind), [cli]);
        }
    }

    #[test]
    fn sandbox_012_volumes_go_on_the_config_drive_read_only_mounted_from_it_writable_copied() {
        let volumes = [
            ContainerMount::read_only("/lab/r1.conf", "/mnt/flash/startup-config"),
            ContainerMount::writable("/lab/certs", "/etc/a:b"),
        ];
        let guest = guest_volumes(&volumes);
        assert_eq!(
            guest.mounts,
            [ContainerMount::read_only(
                "/media/config/sherpa/volumes/0",
                "/mnt/flash/startup-config"
            )]
        );
        assert_eq!(
            guest.writable,
            [WritableVolume {
                source: "/media/config/sherpa/volumes/1".to_owned(),
                destination: "/etc/a:b".to_owned(),
            }]
        );
        let on_drive: Vec<(&str, &str)> = guest
            .drive
            .iter()
            .map(|volume| (volume.source.as_str(), volume.on_drive.as_str()))
            .collect();
        assert_eq!(
            on_drive,
            [
                ("/lab/r1.conf", "sherpa/volumes/0"),
                ("/lab/certs", "sherpa/volumes/1")
            ]
        );
    }

    #[test]
    fn sandbox_052_the_config_drive_holds_ignition_the_container_body_and_each_volume_file_and_folder()
     {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("r1.conf");
        fs::write(&config, "hostname r1\n").unwrap();
        let certs = dir.path().join("certs");
        fs::create_dir_all(certs.join("sub")).unwrap();
        fs::write(certs.join("sub/ca.crt"), "CA").unwrap();
        let stage = dir.path().join("stage");
        let volumes = guest_volumes(&[
            ContainerMount::read_only(config.to_str().unwrap(), "/a"),
            ContainerMount::writable(certs.to_str().unwrap(), "/b"),
        ]);
        stage_config_drive(&stage, "{}", Some(r#"{"Image":"a"}"#), &volumes.drive).unwrap();
        assert_eq!(
            fs::read_to_string(stage.join("openstack/latest/user_data")).unwrap(),
            "{}"
        );
        assert_eq!(
            fs::read_to_string(stage.join("sherpa/container.json")).unwrap(),
            r#"{"Image":"a"}"#
        );
        assert_eq!(
            fs::read_to_string(stage.join("sherpa/volumes/0")).unwrap(),
            "hostname r1\n"
        );
        assert_eq!(
            fs::read_to_string(stage.join("sherpa/volumes/1/sub/ca.crt")).unwrap(),
            "CA"
        );
        let missing = guest_volumes(&[ContainerMount::read_only("/no/such/file", "/c")]);
        assert!(stage_config_drive(&dir.path().join("other"), "{}", None, &missing.drive).is_err());
    }
}
