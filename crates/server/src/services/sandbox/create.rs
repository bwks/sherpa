//! Creating a sandbox: its config drive, its disks on the base image and its image's Docker
//! store, and its domain, and watching for the status it reports once it has started its
//! container.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use askama::Template;
use bollard::Docker;
use libvirt::Qemu;
use tokio::task;
use tracing::instrument;

use container::ContainerSpec;
use shared::data::{DbVmSandbox, Dns, InterfaceType, ModelKind, SherpaNetwork};
use shared::konst::SHERPA_STORAGE_POOL_PATH;
use template::{DomainTuning, FlatcarDomain, SandboxDomain, SandboxGuest, SandboxOwnNic};

use super::base::SandboxBase;
use super::guest::{
    console_command, guest_container_body, guest_nics, guest_volumes, management_addresses,
    write_config_drive,
};
use super::status::StatusWatch;
use super::store::ensure_docker_store;
use crate::daemon::sandbox_disks::SandboxDiskLocks;
use crate::konst::SHERPA_MANAGEMENT_NETWORK_NAME;
use crate::services::console;
use crate::services::error::{SandboxRefusal, ServiceError};
use crate::services::node_resources::domain_name;
use crate::services::p2p_links::ContainerVeth;
use crate::services::progress::ProgressSender;

/// The memory a sandbox has beyond its container's.
pub(crate) const SANDBOX_MEMORY_OVERHEAD_MIB: u16 = 1024;
/// The folder in the node's lab folder its config drive is staged in.
const CONFIG_DRIVE_STAGE: &str = "config-drive";

/// What sandboxes are made with: the server's Docker, libvirt and QEMU binary, the base image
/// when the operation has a sandbox, the Docker store locks, and the operation's progress.
#[derive(Clone)]
pub(crate) struct SandboxHost {
    pub(crate) docker: Arc<Docker>,
    pub(crate) qemu: Arc<Qemu>,
    pub(crate) qemu_bin: String,
    pub(crate) base: Option<SandboxBase>,
    pub(crate) disk_locks: SandboxDiskLocks,
    pub(crate) progress: ProgressSender,
}

/// What a sandbox is created from.
pub(crate) struct SandboxCreate {
    pub(crate) lab_id: String,
    pub(crate) lab_dir: String,
    pub(crate) node_name: String,
    pub(crate) node_index: u16,
    pub(crate) model_kind: ModelKind,
    pub(crate) spec: ContainerSpec,
    pub(crate) dns: Dns,
    pub(crate) memory: u16,
    pub(crate) vcpus: u8,
    pub(crate) tuning: DomainTuning,
    pub(crate) veths: Vec<ContainerVeth>,
    pub(crate) mgmt_net: SherpaNetwork,
    pub(crate) ipv4: Ipv4Addr,
    pub(crate) ipv6: Option<Ipv6Addr>,
    pub(crate) own_address: DbVmSandbox,
}

/// A disk a sandbox's domain boots with, made before the domain is defined.
pub(super) enum DomainDisk {
    Overlay { base: String, path: String },
    Blank { path: String, size_gb: u16 },
}

/// The storage pool path of volume `volume`.
pub(super) fn pool_path(volume: &str) -> String {
    format!("{SHERPA_STORAGE_POOL_PATH}/{volume}")
}

/// The storage pool volume of sandbox `device`'s disk, an overlay on the base image.
fn disk_name(device: &str) -> String {
    format!("{device}-hdd.qcow2")
}

/// The storage pool volume of sandbox `device`'s Docker disk, an overlay on its image's store.
fn docker_disk_name(device: &str) -> String {
    format!("{device}-docker.qcow2")
}

/// The storage pool volume of sandbox `device`'s config drive.
fn config_drive_name(device: &str) -> String {
    format!("{device}.iso")
}

/// The domain tuning of sandbox node `node`: what a VM with its CPU, NUMA, boot disk and
/// interface settings gets, its management NIC virtio.
pub(crate) fn sandbox_tuning(node: &topology::NodeExpanded) -> DomainTuning {
    DomainTuning::from_node_settings(
        &node.cpu,
        &node.numa,
        &node.boot_disk,
        &node.interfaces,
        &InterfaceType::Virtio,
    )
}

/// Create the node's VM and start it, and return the watch on the status it reports.
///
/// # Errors
///
/// Fails when the base image is missing, the image's store cannot be made, the
/// config drive cannot be written, or libvirt cannot create the disks or define and start the
/// domain.
#[instrument(skip(host, create), fields(lab_id = %create.lab_id, node_name = %create.node_name))]
pub(crate) async fn create_sandbox(
    host: &SandboxHost,
    create: SandboxCreate,
) -> Result<StatusWatch> {
    let base = host
        .base
        .as_ref()
        .ok_or(ServiceError::Sandbox(SandboxRefusal::BaseImageMissing))?;
    let store = ensure_docker_store(host, &base.disk, &create.spec.image).await?;
    let device = domain_name(&create.node_name, &create.lab_id);
    let nics = guest_nics(create.model_kind, &create.veths)?;
    let volumes = guest_volumes(&create.spec.volumes);
    let container_body = guest_container_body(&create.spec, volumes.mounts, &create.dns)?;
    let docker_api_version = container::docker_api_version();
    let ignition = SandboxGuest {
        hostname: &create.node_name,
        container: &create.spec.name,
        interfaces: &nics.interfaces,
        placeholders: &nics.placeholders,
        management: &management_addresses(&create.mgmt_net, create.ipv4, create.ipv6),
        own_nic: &SandboxOwnNic {
            mac: create.own_address.mac.clone(),
            addresses: management_addresses(
                &create.mgmt_net,
                create.own_address.ipv4,
                create.own_address.ipv6,
            ),
        },
        console_command: console_command(create.model_kind),
        docker_api_version: &docker_api_version,
        writable: &volumes.writable,
    }
    .ignition_json()?;
    let stage = Path::new(&create.lab_dir)
        .join(&create.node_name)
        .join(CONFIG_DRIVE_STAGE);
    let drive = config_drive_name(&device);
    let config_drive = task::spawn_blocking(move || {
        write_config_drive(
            &stage,
            &drive,
            &ignition,
            Some(&container_body),
            &volumes.drive,
        )
    })
    .await
    .context("The sandbox config drive write panicked")??;
    let status_socket =
        console::lab_socket(&create.lab_id, &format!("{}.status", create.node_index))?;
    let disk = pool_path(&disk_name(&device));
    let docker_disk = pool_path(&docker_disk_name(&device));
    let xml = FlatcarDomain {
        name: device.clone(),
        memory: create.memory,
        vcpus: create.vcpus,
        qemu_bin: host.qemu_bin.clone(),
        disk: disk.clone(),
        config_drive,
    }
    .sandbox(SandboxDomain {
        docker_disk: docker_disk.clone(),
        lab_id: create.lab_id.clone(),
        management_network: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{}", create.lab_id),
        interfaces: nics.domain,
        own_mac: create.own_address.mac.clone(),
        serial_socket: console::console_sockets(&create.lab_id, create.node_index)?.serial,
        status_socket: status_socket.clone(),
        tuning: create.tuning,
    })
    .render()
    .with_context(|| format!("Cannot render the domain of sandbox {device}"))?;
    let disks = vec![
        DomainDisk::Overlay {
            base: base.disk.clone(),
            path: disk,
        },
        DomainDisk::Overlay {
            base: store,
            path: docker_disk,
        },
    ];
    start_domain(&host.qemu, &device, xml, disks).await?;
    StatusWatch::connect(&status_socket, &device).await
}

/// Make `disks`, and define and start domain `name` from its XML `xml`.
///
/// # Errors
///
/// Fails when a disk cannot be made, or libvirt cannot define or start the domain.
#[instrument(skip(qemu, xml, disks), level = "debug")]
pub(super) async fn start_domain(
    qemu: &Arc<Qemu>,
    name: &str,
    xml: String,
    disks: Vec<DomainDisk>,
) -> Result<()> {
    let qemu = Arc::clone(qemu);
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        for disk in &disks {
            match disk {
                DomainDisk::Overlay { base, path } => {
                    libvirt::create_overlay(&conn, base, path, None)?;
                }
                DomainDisk::Blank { path, size_gb } => {
                    libvirt::create_blank_qcow2(&conn, path, *size_gb)?;
                }
            }
        }
        libvirt::create_vm(&conn, &xml)?;
        Ok(())
    })
    .await
    .context("The sandbox start panicked")?
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::Duration;

    use ipnet::Ipv4Net;
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio::time;
    use virt::network::Network;

    use shared::data::{ContainerNetworkAttachment, NetworkV4, NodeKind, NodeModel, RecordId};
    use shared::konst::{QEMU_BIN, SHERPA_IMAGES_PATH, VM_DISK_FILENAME};
    use shared::util::default_dns;

    use super::*;
    use crate::konst::{SHERPA_CONSOLE_DIR, SHERPA_SANDBOX_RUN_DIR};
    use crate::services::image_lifecycle::lease_image;
    use crate::services::node_ops;

    /// A lab's management network `198.18.250.0/24`, removed when dropped.
    struct TestNetwork {
        qemu: Arc<Qemu>,
        name: String,
        network: SherpaNetwork,
    }

    impl TestNetwork {
        fn create(qemu: &Arc<Qemu>, lab_id: &str) -> Self {
            let prefix: Ipv4Net = "198.18.250.0/24".parse().unwrap();
            let name = format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}");
            libvirt::NatNetwork {
                network_name: name.clone(),
                bridge_name: format!("brm-{lab_id}"),
                ipv4_address: Ipv4Addr::new(198, 18, 250, 1),
                ipv4_netmask: prefix.netmask(),
                ipv6_address: None,
                ipv6_prefix_length: None,
            }
            .create(&qemu.connect().unwrap())
            .unwrap();
            Self {
                qemu: Arc::clone(qemu),
                name,
                network: SherpaNetwork {
                    v4: NetworkV4 {
                        prefix,
                        first: Ipv4Addr::new(198, 18, 250, 1),
                        last: prefix.broadcast(),
                        boot_server: Ipv4Addr::new(198, 18, 250, 2),
                        network: prefix.network(),
                        subnet_mask: prefix.netmask(),
                        hostmask: prefix.hostmask(),
                        prefix_length: prefix.prefix_len(),
                    },
                    v6: None,
                },
            }
        }
    }

    impl Drop for TestNetwork {
        fn drop(&mut self) {
            let conn = self.qemu.connect().unwrap();
            if let Ok(network) = Network::lookup_by_name(&conn, &self.name) {
                let _ = network.destroy();
                let _ = network.undefine();
            }
        }
    }

    /// The base image of the installed server: the latest `flatcar_sandbox` version in its
    /// images folder.
    async fn installed_base() -> SandboxBase {
        let dir = Path::new(SHERPA_IMAGES_PATH).join(NodeModel::FlatcarSandbox.to_string());
        let version = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .max()
            .unwrap();
        let model_kind = ModelKind::default_for(NodeModel::FlatcarSandbox);
        SandboxBase {
            disk: dir
                .join(&version)
                .join(VM_DISK_FILENAME)
                .to_str()
                .unwrap()
                .to_owned(),
            _lease: lease_image(model_kind, &version).await,
        }
    }

    /// A sandbox of a `sonic_linux` node on `busybox:1.37` in lab `lab_id`, with the lab's
    /// management network and console folder, all removed when dropped.
    struct TestSandbox {
        qemu: Arc<Qemu>,
        lab_id: &'static str,
        lab_dir: TempDir,
        network: TestNetwork,
    }

    impl TestSandbox {
        fn new(lab_id: &'static str) -> Self {
            let qemu = Arc::new(Qemu::default());
            let network = TestNetwork::create(&qemu, lab_id);
            console::create_console_dir(lab_id).unwrap();
            Self {
                qemu,
                lab_id,
                lab_dir: TempDir::new().unwrap(),
                network,
            }
        }

        fn domain(&self) -> String {
            domain_name("sonic1", self.lab_id)
        }

        async fn create(&self) -> Result<StatusWatch> {
            let host = SandboxHost {
                docker: Arc::new(Docker::connect_with_local_defaults().unwrap()),
                qemu: Arc::clone(&self.qemu),
                qemu_bin: QEMU_BIN.to_owned(),
                base: Some(installed_base().await),
                disk_locks: SandboxDiskLocks::default(),
                progress: ProgressSender::new(mpsc::unbounded_channel().0),
            };
            let spec = node_ops::container_spec(
                self.lab_id,
                "sonic1",
                "1.37",
                node_ops::ContainerSettings {
                    image: "busybox".to_owned(),
                    env_vars: Vec::new(),
                    volumes: Vec::new(),
                    commands: vec!["sleep".to_owned(), "infinity".to_owned()],
                    capabilities: Vec::new(),
                    privileged: true,
                    shm_size: None,
                    user: None,
                },
                ContainerNetworkAttachment {
                    name: self.network.name.clone(),
                    ipv4_address: None,
                    ipv6_address: None,
                    linux_interface_name: None,
                    admin_down: false,
                },
            );
            create_sandbox(
                &host,
                SandboxCreate {
                    lab_id: self.lab_id.to_owned(),
                    lab_dir: self.lab_dir.path().to_str().unwrap().to_owned(),
                    node_name: "sonic1".to_owned(),
                    node_index: 1,
                    model_kind: ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container))
                        .unwrap(),
                    spec,
                    dns: default_dns(&self.network.network),
                    memory: 1024,
                    vcpus: 1,
                    tuning: DomainTuning::default(),
                    veths: Vec::new(),
                    mgmt_net: self.network.network.clone(),
                    ipv4: Ipv4Addr::new(198, 18, 250, 23),
                    ipv6: None,
                    own_address: DbVmSandbox {
                        node: RecordId::new("node", "sonic1"),
                        ipv4: Ipv4Addr::new(198, 18, 250, 24),
                        ipv6: None,
                        mac: "52:54:00:5b:00:01".to_owned(),
                    },
                },
            )
            .await
        }

        fn lab_volumes(&self) -> Vec<String> {
            libvirt::list_lab_volumes(&self.qemu.connect().unwrap(), self.lab_id).unwrap()
        }
    }

    impl Drop for TestSandbox {
        fn drop(&mut self) {
            let conn = self.qemu.connect().unwrap();
            let _ = libvirt::remove_domain(&conn, &self.domain());
            for volume in self.lab_volumes() {
                let _ = libvirt::delete_disk(&conn, &volume);
            }
            let _ = console::remove_console_dir(Path::new(SHERPA_CONSOLE_DIR), self.lab_id);
            // Made by root here, the folder would refuse sherpad later; empty, it is made again.
            let _ = fs::remove_dir(SHERPA_SANDBOX_RUN_DIR);
        }
    }

    #[tokio::test]
    #[ignore = "Requires libvirt, sherpa-pool, Docker with busybox:1.37 and a flatcar_sandbox image; run as root"]
    async fn sandbox_019_a_sandbox_boots_on_its_store_and_its_unit_starts_the_container() {
        let vm = TestSandbox::new("nvtest19");
        let mut watch = vm.create().await.unwrap();
        time::timeout(Duration::from_secs(300), watch.wait_started())
            .await
            .unwrap()
            .unwrap();
        node_ops::destroy_vm_node(
            Arc::new(vm.qemu.connect().unwrap()),
            "sonic1",
            vm.lab_id,
            1,
            0,
        )
        .await
        .unwrap();
        assert_eq!(vm.lab_volumes(), Vec::<String>::new());
    }
}
