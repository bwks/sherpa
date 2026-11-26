//! Creating a node VM: its config drive, its disks on the base image and its image's Docker
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
use shared::data::{Dns, ModelKind, SherpaNetwork};
use shared::konst::SHERPA_STORAGE_POOL_PATH;
use template::{NodeVmDomainTemplate, NodeVmGuest, NodeVmManagement};

use super::base::{base_image, base_kernel, kernel_cmdline, kernel_usr_hash, require_base_image};
use super::guest::{
    console_command, guest_create_args, guest_nics, guest_volumes, management_addresses,
    write_config_drive,
};
use super::status::StatusWatch;
use super::store::ensure_docker_store;
use crate::daemon::docker_stores::DockerStoreLocks;
use crate::konst::SHERPA_MANAGEMENT_NETWORK_NAME;
use crate::services::console;
use crate::services::node_resources::domain_name;
use crate::services::p2p_links::ContainerVeth;

/// The memory a node VM has beyond its container's.
pub(crate) const NODE_VM_MEMORY_OVERHEAD_MIB: u16 = 1024;
/// The folder in the node's lab folder its config drive is staged in.
const CONFIG_DRIVE_STAGE: &str = "config-drive";

/// What node VMs are made with: the server's Docker and libvirt, the images folder, and the
/// Docker store locks.
#[derive(Clone)]
pub(crate) struct NodeVmHost {
    pub(crate) docker: Arc<Docker>,
    pub(crate) qemu: Arc<Qemu>,
    pub(crate) images_dir: String,
    pub(crate) store_locks: DockerStoreLocks,
}

/// What a node VM is created from.
pub(crate) struct NodeVmCreate {
    pub(crate) lab_id: String,
    pub(crate) lab_dir: String,
    pub(crate) node_name: String,
    pub(crate) node_index: u16,
    pub(crate) model_kind: ModelKind,
    pub(crate) spec: ContainerSpec,
    pub(crate) dns: Dns,
    pub(crate) memory_mib: u64,
    pub(crate) vcpus: u8,
    pub(crate) veths: Vec<ContainerVeth>,
    pub(crate) mgmt_net: SherpaNetwork,
    pub(crate) ipv4: Ipv4Addr,
    pub(crate) ipv6: Option<Ipv6Addr>,
}

/// A disk a node VM's domain boots with, made before the domain is defined.
pub(super) enum DomainDisk {
    Overlay { base: String, path: String },
    Blank { path: String, size_gb: u16 },
}

/// The storage pool volume of node VM `device`'s disk, an overlay on the base image.
fn disk_name(device: &str) -> String {
    format!("{device}-hdd.qcow2")
}

/// The storage pool volume of node VM `device`'s Docker disk, an overlay on its image's store.
fn docker_disk_name(device: &str) -> String {
    format!("{device}-docker.qcow2")
}

/// The storage pool volume of node VM `device`'s config drive.
fn config_drive_name(device: &str) -> String {
    format!("{device}.iso")
}

/// Create the node's VM and start it, and return the watch on the status it reports.
///
/// # Errors
///
/// Fails when the base image is missing, the image's store cannot be made, the config drive
/// cannot be written, or libvirt cannot create the disks or define and start the domain.
#[instrument(skip(host, create), fields(lab_id = %create.lab_id, node_name = %create.node_name))]
pub(crate) async fn create_node_vm(host: &NodeVmHost, create: NodeVmCreate) -> Result<StatusWatch> {
    require_base_image(&host.images_dir, &[create.model_kind])?;
    let usr_hash = kernel_usr_hash(&host.images_dir)?;
    let store = ensure_docker_store(host, &create.lab_id, &create.spec.image).await?;
    let device = domain_name(&create.node_name, &create.lab_id);
    let nics = guest_nics(create.model_kind, &create.veths)?;
    let volumes = guest_volumes(&create.spec.volumes);
    let create_args = guest_create_args(&create.spec, volumes.mounts, &create.dns);
    let ignition = NodeVmGuest {
        hostname: &create.node_name,
        container: &create.spec.name,
        interfaces: &nics.interfaces,
        placeholders: &nics.placeholders,
        management: &management_addresses(&create.mgmt_net, create.ipv4, create.ipv6),
        console_command: console_command(create.model_kind),
        create_args: &create_args,
        writable: &volumes.writable,
    }
    .ignition_json()?;
    let config_drive = write_config_drive(
        &Path::new(&create.lab_dir)
            .join(&create.node_name)
            .join(CONFIG_DRIVE_STAGE),
        &config_drive_name(&device),
        &ignition,
        &volumes.drive,
    )?;
    let status_socket =
        console::lab_socket(&create.lab_id, &format!("{}.status", create.node_index))?;
    let domain = NodeVmDomainTemplate {
        name: device.clone(),
        memory_mib: create.memory_mib,
        vcpus: create.vcpus,
        kernel: base_kernel(&host.images_dir),
        cmdline: kernel_cmdline(&usr_hash, true),
        disk: format!("{SHERPA_STORAGE_POOL_PATH}/{}", disk_name(&device)),
        docker_disk: format!("{SHERPA_STORAGE_POOL_PATH}/{}", docker_disk_name(&device)),
        image_archive: None,
        config_drive,
        management: Some(NodeVmManagement {
            network: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{}", create.lab_id),
            mac: nics.management_mac,
        }),
        nics: nics.nics,
        status_socket: status_socket.clone(),
        serial_socket: Some(console::console_sockets(&create.lab_id, create.node_index)?.serial),
    };
    let disks = vec![
        DomainDisk::Overlay {
            base: base_image(&host.images_dir),
            path: domain.disk.clone(),
        },
        DomainDisk::Overlay {
            base: store,
            path: domain.docker_disk.clone(),
        },
    ];
    start_domain(&host.qemu, domain, disks, kernel_cmdline(&usr_hash, false)).await?;
    StatusWatch::connect(&status_socket, &device).await
}

/// Make `disks`, define and start `domain`, and define it again with `next_cmdline` for its
/// later boots, so only the boot it starts now runs Ignition.
///
/// # Errors
///
/// Fails when the domain XML cannot be rendered, a disk cannot be made, or libvirt cannot
/// define, start or redefine the domain.
#[instrument(skip_all, fields(name = %domain.name), level = "debug")]
pub(super) async fn start_domain(
    qemu: &Arc<Qemu>,
    domain: NodeVmDomainTemplate,
    disks: Vec<DomainDisk>,
    next_cmdline: String,
) -> Result<()> {
    let xml = domain
        .render()
        .with_context(|| format!("Cannot render the domain of node VM {}", domain.name))?;
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
        libvirt::set_node_vm_cmdline(&conn, &domain.name, &next_cmdline)
    })
    .await
    .context("The node VM start panicked")?
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ipnet::Ipv4Net;
    use tempfile::TempDir;
    use tokio::time;
    use virt::domain::Domain;
    use virt::network::Network;
    use virt::sys::VIR_DOMAIN_XML_INACTIVE;

    use shared::data::{ContainerNetworkAttachment, NetworkV4, NodeKind, NodeModel};
    use shared::konst::SHERPA_IMAGES_PATH;
    use shared::util::default_dns;

    use super::*;
    use crate::konst::SHERPA_CONSOLE_DIR;
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

    /// A node VM of a `sonic_linux` node on `busybox:1.37` in lab `lab_id`, with the lab's
    /// management network and console folder, all removed when dropped.
    struct TestNodeVm {
        qemu: Arc<Qemu>,
        lab_id: &'static str,
        lab_dir: TempDir,
        network: TestNetwork,
    }

    impl TestNodeVm {
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
            let host = NodeVmHost {
                docker: Arc::new(Docker::connect_with_local_defaults().unwrap()),
                qemu: Arc::clone(&self.qemu),
                images_dir: SHERPA_IMAGES_PATH.to_owned(),
                store_locks: DockerStoreLocks::default(),
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
            create_node_vm(
                &host,
                NodeVmCreate {
                    lab_id: self.lab_id.to_owned(),
                    lab_dir: self.lab_dir.path().to_str().unwrap().to_owned(),
                    node_name: "sonic1".to_owned(),
                    node_index: 1,
                    model_kind: ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container))
                        .unwrap(),
                    spec,
                    dns: default_dns(&self.network.network),
                    memory_mib: 1024,
                    vcpus: 1,
                    veths: Vec::new(),
                    mgmt_net: self.network.network.clone(),
                    ipv4: Ipv4Addr::new(198, 18, 250, 23),
                    ipv6: None,
                },
            )
            .await
        }

        fn domain_xml(&self, flags: u32) -> String {
            Domain::lookup_by_name(&self.qemu.connect().unwrap(), &self.domain())
                .unwrap()
                .get_xml_desc(flags)
                .unwrap()
        }

        fn lab_volumes(&self) -> Vec<String> {
            libvirt::list_lab_volumes(&self.qemu.connect().unwrap(), self.lab_id).unwrap()
        }
    }

    impl Drop for TestNodeVm {
        fn drop(&mut self) {
            let conn = self.qemu.connect().unwrap();
            let _ = libvirt::remove_domain(&conn, &self.domain());
            for volume in self.lab_volumes() {
                let _ = libvirt::delete_disk(&conn, &volume);
            }
            let _ = console::remove_console_dir(Path::new(SHERPA_CONSOLE_DIR), self.lab_id);
        }
    }

    #[tokio::test]
    #[ignore = "Requires libvirt, sherpa-pool, Docker with busybox:1.37 and the node VM base image; run as root"]
    async fn node_vm_019_a_node_vm_boots_on_its_store_and_its_unit_starts_the_container() {
        let vm = TestNodeVm::new("nvtest19");
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

    #[tokio::test]
    #[ignore = "Requires libvirt, sherpa-pool, Docker with busybox:1.37 and the node VM base image; run as root"]
    async fn node_vm_063_a_started_node_vm_is_defined_for_later_boots_without_ignition_whether_or_not_its_status_is_read()
     {
        let vm = TestNodeVm::new("nvtest63");
        let watch = vm.create().await.unwrap();
        drop(watch);
        assert!(vm.domain_xml(0).contains("flatcar.first_boot=detected"));
        let inactive = vm.domain_xml(VIR_DOMAIN_XML_INACTIVE);
        assert!(inactive.contains("<cmdline>"), "{inactive}");
        assert!(!inactive.contains("flatcar.first_boot"), "{inactive}");
    }
}
