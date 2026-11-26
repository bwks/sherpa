//! Node VMs: the container models that need root-like privileges run in a Flatcar VM of their
//! own, on that VM's Docker, so the privileges reach the VM's kernel and not the server's. The
//! VM is a libvirt domain with the node's device name; its data interfaces are taps named like
//! the node's link ends, which the guest moves into the container. See
//! `docs/architecture/NODE_VM.md`.

mod ignition;
mod store;

use std::fs::{self, File};
use std::io::{ErrorKind, Read};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::str;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bollard::Docker;
use container::{BridgeSubnet, ContainerSpec, NodeVmFile, NodeVmTls};
use libvirt::{NodeVm, NodeVmManagement, NodeVmNic, Qemu};
use thiserror::Error;
use tokio::{task, time};
use tracing::instrument;

use shared::data::{ModelKind, NodeModel, SherpaNetwork};
use shared::konst::{KVM_OUI, SHERPA_STORAGE_POOL_PATH};
use shared::util;

use self::ignition::{GuestConfig, ignition_json};
use self::store::ensure_docker_store;
use crate::konst::{NODE_CONFIG_DIR_MODE, NODE_CONFIG_FILE_MODE, SHERPA_MANAGEMENT_NETWORK_NAME};
use crate::services::console;
use crate::services::error::ServiceError;
use crate::services::p2p_links::ContainerVeth;
use crate::tls::generator::{
    CaSpec, ClientSubject, KeyAlgorithm, ServerSubject, generate_ca, issue_client_certificate,
    issue_server_certificate,
};

/// The Flatcar Container Linux release node VMs boot. The install and update scripts download
/// it and check it against its published SHA-512; see `docs/architecture/NODE_VM.md`.
pub(crate) const FLATCAR_NODE_VM_VERSION: &str = "4757.2.4";
/// The folder under the images folder that holds the node VM base image.
pub(crate) const NODE_VM_IMAGE_DIR: &str = "node_vm";
/// The vsock port of a node VM's Docker API.
pub(crate) const NODE_VM_DOCKER_PORT: u32 = 2376;
/// The name of a node VM's management NIC in the VM, the parent of the container's macvlan.
pub(crate) const NODE_VM_MANAGEMENT_INTERFACE: &str = "sherpa0";
/// The memory a node VM needs beyond its container's: Flatcar, Docker and the page cache of
/// the image.
pub(crate) const NODE_VM_MEMORY_OVERHEAD_MIB: u16 = 1024;
/// The GPT partition UUID of Flatcar's first `/usr` partition, which a node VM always boots:
/// updates are off, so the second is never used.
const FLATCAR_USR_A_PARTUUID: &str = "7130c94a-213a-4e5a-8e26-6cce9662f132";
/// Where in Flatcar's kernel image the hash of its `/usr` partition is, as hex.
const KERNEL_USR_HASH_OFFSET: usize = 64;
/// The length of that hash in hex digits.
const KERNEL_USR_HASH_LEN: usize = 64;
/// How long the node VM's certificates are valid; a lab outliving them can no longer reach the
/// VM's Docker and has to redeploy the node.
const NODE_VM_CERT_VALIDITY_DAYS: u32 = 3650;
/// The longest a single Docker request to a node VM may take: copying an image into it.
const NODE_VM_DOCKER_TIMEOUT_SECS: u64 = 1800;
/// How long a node VM's Docker has to answer after its domain starts.
const NODE_VM_DOCKER_WAIT: Duration = Duration::from_secs(300);
/// The pause between two checks of whether a node VM's Docker answers.
const NODE_VM_DOCKER_POLL: Duration = Duration::from_secs(2);
/// The folder in the node's lab folder that holds sherpad's certificates for the VM's Docker.
const NODE_VM_STATE_DIR: &str = "node-vm";
/// The volume label Flatcar's Ignition looks for on a config drive.
const CONFIG_DRIVE_LABEL: &str = "config-2";
/// Where Ignition reads its config on a config drive.
const CONFIG_DRIVE_USER_DATA: &str = "openstack/latest/user_data";
const CA_FILE: &str = "ca.pem";
const CLIENT_CERT_FILE: &str = "client.pem";
const CLIENT_KEY_FILE: &str = "client-key.pem";
/// The organization of a node VM's certificates.
const NODE_VM_CERT_ORGANIZATION: &str = "Sherpa node VM";

/// A node VM operation's failure that a caller tells apart from the others.
#[derive(Debug, Error)]
pub(crate) enum NodeVmError {
    #[error("Node VM {name} is not running")]
    NotRunning { name: String },

    #[error("Docker in node VM {name} did not answer within {seconds} seconds")]
    DockerNotReady { name: String, seconds: u64 },

    #[error("Volume '{0}' is not source:destination")]
    VolumeSpec(String),

    #[error("The node VM kernel {path} has no /usr hash")]
    KernelUsrHash { path: String },
}

/// What a node VM is created from.
pub(crate) struct NodeVmCreate<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) lab_dir: &'a str,
    pub(crate) images_dir: &'a str,
    pub(crate) node_name: &'a str,
    pub(crate) node_index: u16,
    pub(crate) model_kind: ModelKind,
    pub(crate) image: String,
    pub(crate) memory_mib: u64,
    pub(crate) vcpus: u8,
    pub(crate) veths: Vec<&'a ContainerVeth>,
}

/// The domain name of node `node_name`'s VM in lab `lab_id`: its device name.
pub(crate) fn device_name(node_name: &str, lab_id: &str) -> String {
    format!("{node_name}-{lab_id}")
}

/// The node VM base image in images folder `images_dir`.
pub(crate) fn base_image(images_dir: &str) -> String {
    format!("{images_dir}/{NODE_VM_IMAGE_DIR}/flatcar-{FLATCAR_NODE_VM_VERSION}.qcow2")
}

/// The kernel of the node VM base image in images folder `images_dir`, which a node VM boots
/// directly.
pub(crate) fn base_kernel(images_dir: &str) -> String {
    format!("{images_dir}/{NODE_VM_IMAGE_DIR}/flatcar-{FLATCAR_NODE_VM_VERSION}.vmlinuz")
}

/// Refuse when one of `model_kinds` runs in a node VM and the base image or its kernel is
/// missing from images folder `images_dir`, before anything is created.
///
/// # Errors
///
/// Fails with [`ServiceError::NodeVmBaseImageMissing`] when the base image or its kernel is
/// needed and not there.
pub(crate) fn require_base_image(
    images_dir: &str,
    model_kinds: &[ModelKind],
) -> Result<(), ServiceError> {
    if !model_kinds.iter().any(|kind| kind.runs_in_node_vm()) {
        return Ok(());
    }
    for path in [base_image(images_dir), base_kernel(images_dir)] {
        if !Path::new(&path).is_file() {
            return Err(ServiceError::NodeVmBaseImageMissing { path });
        }
    }
    Ok(())
}

/// The hash of the `/usr` partition that Flatcar kernel image `header`, its first bytes, was
/// built with, which the kernel command line passes for dm-verity. GRUB reads it from the same
/// place.
fn usr_hash(header: &[u8]) -> Option<&str> {
    let hash = header.get(KERNEL_USR_HASH_OFFSET..KERNEL_USR_HASH_OFFSET + KERNEL_USR_HASH_LEN)?;
    hash.iter()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        .then(|| str::from_utf8(hash).ok())
        .flatten()
}

/// The hash of the `/usr` partition of the node VM kernel in images folder `images_dir`.
///
/// # Errors
///
/// Fails when the kernel cannot be read or holds no hash.
fn kernel_usr_hash(images_dir: &str) -> Result<String> {
    let path = base_kernel(images_dir);
    let mut header = [0; KERNEL_USR_HASH_OFFSET + KERNEL_USR_HASH_LEN];
    File::open(&path)
        .and_then(|mut file| file.read_exact(&mut header))
        .with_context(|| format!("Cannot read the node VM kernel {path}"))?;
    Ok(usr_hash(&header)
        .ok_or(NodeVmError::KernelUsrHash { path })?
        .to_owned())
}

/// The kernel command line of a node VM: Flatcar's from its GRUB config, without a console, as
/// the serial console is the network OS's and printing the kernel's messages there is slow.
/// `first_boot` makes Ignition run.
fn kernel_cmdline(usr_hash: &str, first_boot: bool) -> String {
    let mut cmdline = format!(
        "mount.usr=/dev/mapper/usr verity.usr=PARTUUID={FLATCAR_USR_A_PARTUUID} rootflags=rw \
         mount.usrflags=ro consoleblank=0 root=LABEL=ROOT flatcar.oem.id=openstack \
         verity.usrhash={usr_hash}"
    );
    if first_boot {
        cmdline.push_str(" flatcar.first_boot=detected");
    }
    cmdline
}

/// The storage pool volume that holds the VM's disk: an overlay on the base image.
fn disk_name(device: &str) -> String {
    format!("{device}-hdd.qcow2")
}

/// The storage pool volume that holds the VM's Docker store: an overlay on the store of the
/// node's image.
fn docker_disk_name(device: &str) -> String {
    format!("{device}-docker.qcow2")
}

/// The storage pool volume of the VM's Ignition config drive.
fn config_drive_name(device: &str) -> String {
    format!("{device}.iso")
}

/// The folder that holds sherpad's certificates for the VM's Docker.
fn state_dir(lab_dir: &str, node_name: &str) -> PathBuf {
    Path::new(lab_dir).join(node_name).join(NODE_VM_STATE_DIR)
}

/// The step a model runs after each start of its container, in the VM: SR Linux's management
/// address flush.
fn on_start_script(model_kind: ModelKind) -> Option<&'static str> {
    match model_kind.model() {
        NodeModel::NokiaSrlinux => Some(include_str!("srlinux-on-start.sh")),
        _ => None,
    }
}

/// The command the VM's serial console runs in the node's container: its network OS CLI.
fn console_command(model_kind: ModelKind) -> &'static [&'static str] {
    match model_kind.model() {
        NodeModel::AristaEos => &["Cli"],
        NodeModel::NokiaSrlinux => &["sr_cli"],
        _ => &["bash"],
    }
}

/// The certificates of a node VM's Docker API: a CA of the VM's own, the server certificate
/// its Docker presents, and the client certificate sherpad presents. The CA's key is dropped
/// once both are issued, so nothing else can be signed for the VM.
struct NodeVmCerts {
    ca: String,
    server_cert: String,
    server_key: String,
    client_cert: String,
    client_key: String,
}

impl NodeVmCerts {
    /// Issue the certificates of the VM with domain name `device`.
    fn issue(device: &str) -> Result<Self> {
        let common_name = format!("Sherpa node VM CA - {device}");
        let spec = CaSpec {
            common_name: Some(&common_name),
            organization: Some(NODE_VM_CERT_ORGANIZATION),
            key_algorithm: KeyAlgorithm::EcdsaP256,
        };
        let ca = generate_ca(&spec, NODE_VM_CERT_VALIDITY_DAYS)?;
        let server = issue_server_certificate(
            &spec,
            &ca.key,
            ServerSubject {
                common_name: device,
                organization: NODE_VM_CERT_ORGANIZATION,
                dns_names: &[device.to_owned()],
                ip_addresses: &[],
            },
            KeyAlgorithm::EcdsaP256,
            NODE_VM_CERT_VALIDITY_DAYS,
        )?;
        let client = issue_client_certificate(
            &spec,
            &ca.key,
            ClientSubject {
                organization: NODE_VM_CERT_ORGANIZATION,
                common_name: Some("sherpad"),
            },
            KeyAlgorithm::EcdsaP256,
            NODE_VM_CERT_VALIDITY_DAYS,
        )?;
        Ok(Self {
            ca: ca.cert,
            server_cert: server.cert,
            server_key: server.key,
            client_cert: client.cert,
            client_key: client.key,
        })
    }
}

/// Write Ignition config `ignition` onto the config drive of VM `device` in the storage pool,
/// staged in `dir`, which is removed again.
fn write_config_drive(dir: &Path, device: &str, ignition: &str) -> Result<String> {
    let stage = dir.join("config-drive");
    if stage.exists() {
        fs::remove_dir_all(&stage).with_context(|| format!("Cannot clear {}", stage.display()))?;
    }
    for folder in [
        stage.clone(),
        stage.join("openstack"),
        stage.join("openstack/latest"),
    ] {
        util::create_dir_with_permissions(&folder, NODE_CONFIG_DIR_MODE)?;
    }
    let user_data = stage.join(CONFIG_DRIVE_USER_DATA);
    util::write_file_with_permissions(&user_data, ignition.as_bytes(), NODE_CONFIG_FILE_MODE)?;
    let drive = format!("{SHERPA_STORAGE_POOL_PATH}/{}", config_drive_name(device));
    let written = iso::create_iso(&stage, &drive, CONFIG_DRIVE_LABEL)
        .with_context(|| format!("Cannot write the config drive of node VM {device}"));
    fs::remove_dir_all(&stage).with_context(|| format!("Cannot remove {}", stage.display()))?;
    written?;
    Ok(drive)
}

/// Create the node's VM and start it: its certificates, its Ignition config drive, its disk on
/// the base image, its Docker store on the store of its image (made first when there is none),
/// and its domain, which boots the base image's kernel directly. Each data interface on a link
/// or bridge is a tap named like the node's link end, and a tap that is a bridge member joins
/// its bridge; the others are made in the VM. What was created is removed again when a later
/// step fails.
///
/// # Errors
///
/// Fails when the base image is missing, a file cannot be written, the image's store cannot be
/// made, or libvirt cannot create the disks or define and start the domain.
#[instrument(skip(host, qemu, create), fields(lab_id = %create.lab_id, node_name = %create.node_name))]
pub(crate) async fn create_node_vm(
    host: &Docker,
    qemu: &Arc<Qemu>,
    create: &NodeVmCreate<'_>,
) -> Result<()> {
    require_base_image(create.images_dir, &[create.model_kind])?;
    let base = base_image(create.images_dir);
    let usr_hash = kernel_usr_hash(create.images_dir)?;
    let store = ensure_docker_store(host, qemu, create.images_dir, &create.image).await?;
    let device = device_name(create.node_name, create.lab_id);
    let dir = state_dir(create.lab_dir, create.node_name);
    util::create_dir_with_permissions(&dir, NODE_CONFIG_DIR_MODE)?;
    let certs = NodeVmCerts::issue(&device)?;
    for (file, contents) in [
        (CA_FILE, &certs.ca),
        (CLIENT_CERT_FILE, &certs.client_cert),
        (CLIENT_KEY_FILE, &certs.client_key),
    ] {
        util::write_file_with_permissions(
            &dir.join(file),
            contents.as_bytes(),
            NODE_CONFIG_FILE_MODE,
        )?;
    }

    let management_mac = util::random_mac(KVM_OUI);
    let mut interfaces = Vec::new();
    let mut placeholders = Vec::new();
    let mut nics = Vec::new();
    for veth in &create.veths {
        let name = util::container_interface_name(create.model_kind, veth.interface_idx)?;
        let mac = util::random_mac(KVM_OUI);
        if veth.admin_down {
            placeholders.push((mac, name));
            continue;
        }
        interfaces.push((mac.clone(), name));
        nics.push(NodeVmNic {
            tap: veth.host_veth.clone(),
            mac,
        });
    }
    let ignition = ignition_json(&GuestConfig {
        hostname: create.node_name,
        container: &device,
        management_mac: &management_mac,
        interfaces: &interfaces,
        placeholders: &placeholders,
        tls: ignition::DockerTls {
            ca: &certs.ca,
            server_cert: &certs.server_cert,
            server_key: &certs.server_key,
        },
        console_command: console_command(create.model_kind),
        on_start: on_start_script(create.model_kind),
    })?;
    let config_drive = write_config_drive(&dir, &device, &ignition)?;
    let vm = NodeVm {
        name: device.clone(),
        memory_mib: create.memory_mib,
        vcpus: create.vcpus,
        kernel: base_kernel(create.images_dir),
        cmdline: kernel_cmdline(&usr_hash, true),
        disk: format!("{SHERPA_STORAGE_POOL_PATH}/{}", disk_name(&device)),
        docker_disk: format!("{SHERPA_STORAGE_POOL_PATH}/{}", docker_disk_name(&device)),
        config_iso: config_drive,
        management: Some(NodeVmManagement {
            network: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{}", create.lab_id),
            mac: management_mac,
        }),
        nics,
        serial_socket: Some(console::console_sockets(create.lab_id, create.node_index)?.serial),
    };
    let started = {
        let qemu = Arc::clone(qemu);
        task::spawn_blocking(move || {
            let conn = qemu.connect()?;
            libvirt::create_overlay(&conn, &base, &vm.disk, None)?;
            libvirt::create_overlay(&conn, &store, &vm.docker_disk, None)?;
            libvirt::create_node_vm(&conn, &vm)
        })
        .await
        .context("The node VM start panicked")
        .and_then(|result| result)
    };
    let joined = async {
        started?;
        for veth in &create.veths {
            if let Some(bridge) = &veth.bridge {
                network::enslave_to_bridge(&veth.host_veth, bridge).await?;
            }
        }
        Ok::<(), anyhow::Error>(())
    }
    .await;
    if let Err(error) = joined {
        if let Err(removal) =
            remove_node_vm(qemu, create.lab_dir, create.node_name, create.lab_id).await
        {
            tracing::warn!(node_name = %create.node_name, error = %removal, "Cannot remove a node VM that did not start");
        }
        return Err(error);
    }
    Ok(())
}

/// The Docker client of running domain `device`, whose Docker presents a certificate of
/// `tls`'s CA and takes `tls`'s client certificate.
///
/// # Errors
///
/// Fails when the domain is not running or libvirt cannot be asked for its vsock ID.
async fn domain_docker(qemu: &Arc<Qemu>, device: &str, tls: &NodeVmTls) -> Result<Docker> {
    let cid = {
        let qemu = Arc::clone(qemu);
        let device = device.to_owned();
        task::spawn_blocking(move || {
            let conn = qemu.connect()?;
            libvirt::node_vm_cid(&conn, &device)
        })
        .await
        .context("The node VM lookup panicked")??
    }
    .ok_or_else(|| NodeVmError::NotRunning {
        name: device.to_owned(),
    })?;
    Ok(container::node_vm_docker(
        cid,
        NODE_VM_DOCKER_PORT,
        tls,
        NODE_VM_DOCKER_TIMEOUT_SECS,
    )?)
}

/// sherpad's certificates for the Docker of node `node_name`'s VM, domain `device`.
///
/// # Errors
///
/// Fails when they cannot be read.
fn node_vm_tls(lab_dir: &str, node_name: &str, device: &str) -> Result<NodeVmTls> {
    let dir = state_dir(lab_dir, node_name);
    let read = |file: &str| {
        fs::read_to_string(dir.join(file))
            .with_context(|| format!("Cannot read {}", dir.join(file).display()))
    };
    Ok(NodeVmTls {
        ca_pem: read(CA_FILE)?,
        client_cert_pem: read(CLIENT_CERT_FILE)?,
        client_key_pem: read(CLIENT_KEY_FILE)?,
        server_name: device.to_owned(),
    })
}

/// A Docker client for the running node VM of node `node_name`.
///
/// # Errors
///
/// Fails when the VM is not running, libvirt cannot be asked for its vsock ID, or sherpad's
/// certificates for it cannot be read.
#[instrument(skip(qemu), level = "debug")]
pub(crate) async fn node_vm_docker(
    qemu: &Arc<Qemu>,
    lab_dir: &str,
    node_name: &str,
    lab_id: &str,
) -> Result<Docker> {
    let device = device_name(node_name, lab_id);
    let tls = node_vm_tls(lab_dir, node_name, &device)?;
    domain_docker(qemu, &device, &tls).await
}

/// Wait until the Docker of running domain `device` answers, and return a client for it.
async fn wait_for_docker(qemu: &Arc<Qemu>, device: &str, tls: &NodeVmTls) -> Result<Docker> {
    let started = Instant::now();
    loop {
        let attempt = async {
            let docker = domain_docker(qemu, device, tls).await?;
            docker.ping().await?;
            Ok::<Docker, anyhow::Error>(docker)
        }
        .await;
        match attempt {
            Ok(docker) => return Ok(docker),
            Err(error) if started.elapsed() < NODE_VM_DOCKER_WAIT => {
                tracing::debug!(%device, %error, "Node VM Docker not answering yet");
            }
            Err(error) => {
                return Err(error).context(NodeVmError::DockerNotReady {
                    name: device.to_owned(),
                    seconds: NODE_VM_DOCKER_WAIT.as_secs(),
                });
            }
        }
        time::sleep(NODE_VM_DOCKER_POLL).await;
    }
}

/// Boot domain `device` without running Ignition from its next start on: its first boot has
/// run it, as its Docker answers.
async fn end_first_boot(qemu: &Arc<Qemu>, device: &str, usr_hash: &str) -> Result<()> {
    let qemu = Arc::clone(qemu);
    let device = device.to_owned();
    let cmdline = kernel_cmdline(usr_hash, false);
    task::spawn_blocking(move || {
        let conn = qemu.connect()?;
        libvirt::set_node_vm_cmdline(&conn, &device, &cmdline)
    })
    .await
    .context("The node VM redefinition panicked")?
}

/// The files of Docker volume specs `source:destination[:ro]`, read from the server, each at
/// its destination in the container; a folder's files go under its destination, recursively.
fn volume_files(volumes: &[String]) -> Result<Vec<NodeVmFile>> {
    let mut files = Vec::new();
    for volume in volumes {
        let mut parts = volume.split(':');
        let (Some(source), Some(destination)) = (parts.next(), parts.next()) else {
            return Err(NodeVmError::VolumeSpec(volume.clone()).into());
        };
        add_files(Path::new(source), destination, &mut files)?;
    }
    Ok(files)
}

/// Add file or folder `source` at `destination`, a folder's entries recursively. Symlinks are
/// not followed.
fn add_files(source: &Path, destination: &str, files: &mut Vec<NodeVmFile>) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Cannot read volume source {}", source.display()))?;
    if metadata.is_dir() {
        for entry in fs::read_dir(source)
            .with_context(|| format!("Cannot read volume folder {}", source.display()))?
        {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            add_files(&entry.path(), &format!("{destination}/{name}"), files)?;
        }
    } else if metadata.is_file() {
        files.push(NodeVmFile {
            path: destination.to_owned(),
            contents: fs::read(source)
                .with_context(|| format!("Cannot read volume source {}", source.display()))?,
            mode: metadata.permissions().mode(),
        });
    }
    Ok(())
}

/// The lab's management subnets as the VM's macvlan network takes them: IPv4, and IPv6 when
/// the lab has it.
fn management_subnets(mgmt_net: &SherpaNetwork) -> (BridgeSubnet, Option<BridgeSubnet>) {
    (
        BridgeSubnet {
            subnet: mgmt_net.v4.prefix.to_string(),
            gateway: Some(mgmt_net.v4.first.to_string()),
        },
        mgmt_net.v6.as_ref().map(|v6| BridgeSubnet {
            subnet: v6.prefix.to_string(),
            gateway: Some(v6.first.to_string()),
        }),
    )
}

/// What the node's container in its VM is started from.
pub(crate) struct NodeContainerStart<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) lab_dir: &'a str,
    pub(crate) images_dir: &'a str,
    pub(crate) node_name: &'a str,
    pub(crate) mgmt_net: &'a SherpaNetwork,
    pub(crate) spec: ContainerSpec,
}

/// Start the node's container in its VM, once the VM's Docker answers: boot the VM without
/// Ignition from then on, copy the node's image into the VM unless its store has it, make the
/// management network, copy the node's files from its
/// volumes into the container, and start it with the settings a container on the server gets.
/// The VM is the limit of its memory and CPUs. The VM runs the model's own step after each
/// start (`on_start_script`). Returns whether it runs.
///
/// # Errors
///
/// Fails when the VM's Docker does not answer in time, a volume cannot be read, or Docker
/// cannot copy the image, make the network or run the container.
#[instrument(skip(host, qemu, start), fields(lab_id = %start.lab_id, node_name = %start.node_name))]
pub(crate) async fn start_node_container(
    host: &Docker,
    qemu: &Arc<Qemu>,
    start: NodeContainerStart<'_>,
) -> Result<bool> {
    let device = device_name(start.node_name, start.lab_id);
    let tls = node_vm_tls(start.lab_dir, start.node_name, &device)?;
    let vm = wait_for_docker(qemu, &device, &tls).await?;
    end_first_boot(qemu, &device, &kernel_usr_hash(start.images_dir)?).await?;
    container::copy_image_to_node_vm(host, &vm, &start.spec.image).await?;
    let (ipv4, ipv6) = management_subnets(start.mgmt_net);
    container::ensure_node_vm_management_network(
        &vm,
        &start.spec.management_network.name,
        NODE_VM_MANAGEMENT_INTERFACE,
        ipv4,
        ipv6,
    )
    .await?;
    let files = volume_files(&start.spec.volumes)?;
    let spec = ContainerSpec {
        volumes: Vec::new(),
        additional_networks: Vec::new(),
        memory_mib: None,
        cpu_count: None,
        ..start.spec
    };
    container::run_node_vm_container(&vm, spec, &util::random_mac(KVM_OUI), &files).await
}

/// Stop and undefine node `node_name`'s VM, delete its disks and config drive, and remove
/// sherpad's certificates for it. What is already gone counts as removed.
///
/// # Errors
///
/// Fails when libvirt cannot stop, undefine or delete what the VM has, or the certificates
/// cannot be removed.
#[instrument(skip(qemu))]
pub(crate) async fn remove_node_vm(
    qemu: &Arc<Qemu>,
    lab_dir: &str,
    node_name: &str,
    lab_id: &str,
) -> Result<()> {
    let device = device_name(node_name, lab_id);
    {
        let qemu = Arc::clone(qemu);
        task::spawn_blocking(move || {
            let conn = qemu.connect()?;
            libvirt::remove_node_vm(&conn, &device)?;
            for volume in [
                disk_name(&device),
                docker_disk_name(&device),
                config_drive_name(&device),
            ] {
                libvirt::delete_volume_if_present(&conn, &volume)?;
            }
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("The node VM removal panicked")??;
    }
    let dir = state_dir(lab_dir, node_name);
    match fs::remove_dir_all(&dir) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(error).with_context(|| format!("Cannot remove {}", dir.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::net::Ipv4Addr;

    use shared::data::NodeKind;
    use shared::konst::SHERPA_IMAGES_PATH;
    use tempfile::TempDir;
    use virt::network::Network;

    use crate::konst::SHERPA_CONSOLE_DIR;

    #[test]
    fn node_vm_012_volume_files_land_at_their_destination_and_folders_recursively() {
        let dir = TempDir::new().unwrap();
        let config = dir.path().join("r1.conf");
        fs::write(&config, "hostname r1\n").unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o640)).unwrap();
        let certs = dir.path().join("certs");
        fs::create_dir_all(certs.join("sub")).unwrap();
        fs::write(certs.join("sub/ca.crt"), "CA").unwrap();
        let files = volume_files(&[
            format!("{}:/mnt/flash/startup-config:ro", config.display()),
            format!("{}:/etc/certs", certs.display()),
        ])
        .unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "/mnt/flash/startup-config");
        assert_eq!(files[0].contents, b"hostname r1\n");
        assert_eq!(files[0].mode & 0o777, 0o640);
        assert_eq!(files[1].path, "/etc/certs/sub/ca.crt");
        assert!(volume_files(&["/only-a-source".to_owned()]).is_err());
    }

    #[test]
    fn node_vm_013_the_console_runs_each_models_cli() {
        let kind = |model| ModelKind::new(model, Some(NodeKind::Container)).unwrap();
        assert_eq!(console_command(kind(NodeModel::AristaEos)), ["Cli"]);
        assert_eq!(console_command(kind(NodeModel::NokiaSrlinux)), ["sr_cli"]);
        assert_eq!(console_command(kind(NodeModel::SonicLinux)), ["bash"]);
    }

    #[test]
    fn node_vm_033_the_usr_hash_is_read_from_the_kernel_header() {
        let hash = "6f38dc650e110fc0c1d2211a9c0ac3d165c7613fd81c9df9a4f3a5e59d6cd0a8";
        let mut header = vec![0u8; KERNEL_USR_HASH_OFFSET];
        header.extend_from_slice(hash.as_bytes());
        assert_eq!(usr_hash(&header), Some(hash));
        assert_eq!(usr_hash(&header[..100]), None);
        header[KERNEL_USR_HASH_OFFSET] = b'G';
        assert_eq!(usr_hash(&header), None);
    }

    #[test]
    fn node_vm_034_the_kernel_boots_without_a_console_and_runs_ignition_on_the_first_boot_only() {
        let first = kernel_cmdline("abc", true);
        assert!(first.contains("verity.usrhash=abc"));
        assert!(first.contains("verity.usr=PARTUUID=7130c94a-213a-4e5a-8e26-6cce9662f132"));
        assert!(first.ends_with(" flatcar.first_boot=detected"));
        assert!(!first.contains("console="));
        assert_eq!(
            kernel_cmdline("abc", false),
            first.trim_end_matches(" flatcar.first_boot=detected")
        );
    }

    #[test]
    fn node_vm_018_a_lab_with_a_node_vm_is_refused_without_the_base_image() {
        let dir = TempDir::new().unwrap();
        let images = dir.path().to_str().unwrap();
        let ceos = ModelKind::new(NodeModel::AristaEos, Some(NodeKind::Container)).unwrap();
        let frr = ModelKind::new(NodeModel::FrrLinux, Some(NodeKind::Container)).unwrap();
        assert!(require_base_image(images, &[frr]).is_ok());
        assert!(matches!(
            require_base_image(images, &[frr, ceos]),
            Err(ServiceError::NodeVmBaseImageMissing { .. })
        ));
        fs::create_dir_all(dir.path().join(NODE_VM_IMAGE_DIR)).unwrap();
        fs::write(base_image(images), b"qcow2").unwrap();
        assert!(matches!(
            require_base_image(images, &[ceos]),
            Err(ServiceError::NodeVmBaseImageMissing { path }) if path == base_kernel(images)
        ));
        fs::write(base_kernel(images), b"kernel").unwrap();
        assert!(require_base_image(images, &[frr, ceos]).is_ok());
    }

    /// A lab management network for a node VM of lab `lab_id`, removed when dropped.
    struct TestNetwork {
        qemu: Arc<Qemu>,
        name: String,
    }

    impl TestNetwork {
        fn create(qemu: &Arc<Qemu>, lab_id: &str) -> Self {
            let name = format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}");
            libvirt::NatNetwork {
                network_name: name.clone(),
                bridge_name: format!("brm-{lab_id}"),
                ipv4_address: Ipv4Addr::new(198, 18, 250, 1),
                ipv4_netmask: Ipv4Addr::new(255, 255, 255, 0),
                ipv6_address: None,
                ipv6_prefix_length: None,
            }
            .create(&qemu.connect().unwrap())
            .unwrap();
            Self {
                qemu: Arc::clone(qemu),
                name,
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

    #[tokio::test]
    #[ignore = "Requires libvirt, sherpa-pool, vhost-vsock and the node VM base image; run as root"]
    async fn node_vm_019_a_node_vm_boots_and_its_docker_answers_only_sherpads_certificate() {
        let qemu = Arc::new(Qemu::default());
        let lab_id = "nvtest19";
        let lab_dir = TempDir::new().unwrap();
        let lab_dir = lab_dir.path().to_str().unwrap();
        let _network = TestNetwork::create(&qemu, lab_id);
        console::create_console_dir(lab_id).unwrap();
        let create = NodeVmCreate {
            lab_id,
            lab_dir,
            images_dir: SHERPA_IMAGES_PATH,
            node_name: "sonic1",
            node_index: 1,
            model_kind: ModelKind::new(NodeModel::SonicLinux, Some(NodeKind::Container)).unwrap(),
            image: "busybox:1.37".to_owned(),
            memory_mib: 1024,
            vcpus: 1,
            veths: vec![],
        };
        let host = Docker::connect_with_local_defaults().unwrap();
        create_node_vm(&host, &qemu, &create).await.unwrap();
        let checked = async {
            let device = device_name("sonic1", lab_id);
            let docker =
                wait_for_docker(&qemu, &device, &node_vm_tls(lab_dir, "sonic1", &device)?).await?;
            anyhow::ensure!(
                docker.inspect_image("busybox:1.37").await.is_ok(),
                "the store has no image"
            );
            docker.ping().await?;
            let cid = {
                let qemu = Arc::clone(&qemu);
                task::spawn_blocking(move || {
                    let conn = qemu.connect()?;
                    libvirt::node_vm_cid(&conn, &device_name("sonic1", lab_id))
                })
                .await??
                .context("no vsock ID")?
            };
            let stranger = NodeVmCerts::issue("sonic1-nvtest19")?;
            let refused = container::node_vm_docker(
                cid,
                NODE_VM_DOCKER_PORT,
                &NodeVmTls {
                    ca_pem: stranger.ca,
                    client_cert_pem: stranger.client_cert,
                    client_key_pem: stranger.client_key,
                    server_name: device_name("sonic1", lab_id),
                },
                10,
            )?;
            anyhow::ensure!(
                refused.ping().await.is_err(),
                "a client of another CA was answered"
            );
            Ok::<(), anyhow::Error>(())
        }
        .await;
        remove_node_vm(&qemu, lab_dir, "sonic1", lab_id)
            .await
            .unwrap();
        console::remove_console_dir(Path::new(SHERPA_CONSOLE_DIR), lab_id).unwrap();
        checked.unwrap();
        let conn = qemu.connect().unwrap();
        assert_eq!(
            libvirt::list_lab_volumes(&conn, lab_id).unwrap(),
            Vec::<String>::new()
        );
        assert!(
            libvirt::node_vm_cid(&conn, &device_name("sonic1", lab_id))
                .unwrap()
                .is_none()
        );
        assert!(!state_dir(lab_dir, "sonic1").exists());
    }

    #[test]
    fn node_vm_014_each_vm_has_a_ca_of_its_own_that_signs_both_ends() {
        let first = NodeVmCerts::issue("ceos1-abc12345").unwrap();
        let second = NodeVmCerts::issue("ceos1-abc12345").unwrap();
        assert_ne!(first.ca, second.ca);
        assert!(first.server_key.contains("PRIVATE KEY"));
        assert!(first.client_cert.contains("BEGIN CERTIFICATE"));
    }
}
