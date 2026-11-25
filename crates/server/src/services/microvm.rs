//! Container nodes whose model runs in a Cloud Hypervisor microVM: the container's spec,
//! turned into a microVM with the same image, command, environment, mounts and interfaces.
//! See `docs/architecture/MICROVM.md`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use bollard::Docker;
use bollard::models::ContainerStateStatusEnum;
use bollard::query_parameters::InspectNetworkOptions;
use container::ContainerSpec;
use container::microvm::{self, GuestConfig, MicroVmNic, MicroVmSpec, MicroVmState};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::daemon::storage::restrict_dir;
use crate::konst::{
    SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_SECRETS_DIR,
};
use crate::services::console;
use crate::services::node_secrets::node_secrets_dir;
use crate::services::p2p_links::ContainerVeth;
use crate::services::progress::ProgressSender;
use shared::data::{ModelKind, NodeModel, StatusKind};
use shared::konst::{CONTAINER_SECRETS_DIR, SHERPA_LABS_PATH};
use shared::util;

const MICROVM_NODE_DIR: &str = "microvm";
const CONFIG_STAGING_DIR: &str = "config";
const CONFIG_ISO: &str = "config.iso";
const NODE_DISK: &str = "node.raw";
/// Suffix of a node's microVM secrets folder in the lab's secrets folder; `~` is in no node
/// name.
const SECRETS_ISO_DIR_SUFFIX: &str = "~microvm";
const SECRETS_STAGING_DIR: &str = "staging";
const SECRETS_ISO: &str = "secrets.iso";
const TAP_WIRING: &str = "taps.toml";
const NODE_DISK_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MGMT_TAP_PREFIX: &str = "cm";
const MGMT_GUEST_INTERFACE: &str = "eth0";
const DEFAULT_MEMORY_MIB: u64 = 4096;
const DEFAULT_CPUS: u8 = 2;
/// Locally administered MACs: the first octet's second bit set.
const MAC_PREFIX: u8 = 0x02;

/// Where a microVM node sits in its lab.
pub(crate) struct MicroVmNode<'a> {
    pub(crate) lab_id: &'a str,
    pub(crate) node_name: &'a str,
    pub(crate) node_index: u16,
    pub(crate) veths: Vec<&'a ContainerVeth>,
}

/// What a microVM node's taps are wired to on the host: its management tap on the lab's
/// management bridge, and each data tap on its bridge or with its carrier taken. Cloud
/// Hypervisor makes the taps again at every boot, so they are wired again from this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct TapWiring {
    mgmt_tap: String,
    mgmt_bridge: String,
    taps: Vec<WiredTap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct WiredTap {
    tap: String,
    bridge: Option<String>,
    down: bool,
}

/// The MAC of NIC `nic` of the node with index `node_index` in lab `lab_id`.
fn nic_mac(lab_id: &str, node_index: u16, nic: u8) -> String {
    let lab = lab_id
        .bytes()
        .fold(0_u8, |hash, byte| hash.wrapping_mul(31).wrapping_add(byte));
    let [high, low] = node_index.to_be_bytes();
    format!("{MAC_PREFIX:02x}:53:{lab:02x}:{high:02x}:{low:02x}:{nic:02x}")
}

/// The host tap of the management NIC of the node with index `node_index`.
fn mgmt_tap(lab_id: &str, node_index: u16) -> String {
    format!("{MGMT_TAP_PREFIX}{node_index}-{lab_id}")
}

/// The folder of a microVM node's disks and config, in its node folder.
fn node_microvm_dir(lab_id: &str, node_name: &str) -> PathBuf {
    Path::new(SHERPA_LABS_PATH)
        .join(lab_id)
        .join(node_name)
        .join(MICROVM_NODE_DIR)
}

/// The management NIC, then one NIC per P2p or bridge interface, named as the container's
/// interfaces are.
fn node_nics(node: &MicroVmNode<'_>, model_kind: ModelKind) -> Result<Vec<MicroVmNic>> {
    let mut nics = vec![MicroVmNic {
        tap: mgmt_tap(node.lab_id, node.node_index),
        mac: nic_mac(node.lab_id, node.node_index, 0),
        guest_name: MGMT_GUEST_INTERFACE.to_owned(),
    }];
    for veth in &node.veths {
        nics.push(MicroVmNic {
            tap: veth.host_veth.clone(),
            mac: nic_mac(node.lab_id, node.node_index, veth.interface_idx),
            guest_name: util::container_interface_name(model_kind, veth.interface_idx)?,
        });
    }
    Ok(nics)
}

/// `base` with each `NAME=value` of `overrides` replacing the one of the same name.
fn merge_env(base: Vec<String>, overrides: &[String]) -> Vec<String> {
    let name = |var: &str| var.split('=').next().unwrap_or_default().to_owned();
    let mut merged: Vec<String> = base
        .into_iter()
        .filter(|var| !overrides.iter().any(|over| name(over) == name(var)))
        .collect();
    merged.extend(overrides.iter().cloned());
    merged
}

/// The lab's management network prefix length and gateway, from its Docker network.
async fn management_network(docker: &Docker, lab_id: &str) -> Result<(u8, Option<String>)> {
    let name = format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}");
    let network = docker
        .inspect_network(&name, None::<InspectNetworkOptions>)
        .await
        .with_context(|| format!("Cannot inspect the management network {name}"))?;
    let config = network
        .ipam
        .and_then(|ipam| ipam.config)
        .unwrap_or_default()
        .into_iter()
        .find(|config| {
            config
                .subnet
                .as_deref()
                .is_some_and(|subnet| subnet.contains('.'))
        })
        .ok_or_else(|| anyhow!("Management network {name} has no IPv4 subnet"))?;
    let prefix = config
        .subnet
        .as_deref()
        .and_then(|subnet| subnet.split('/').nth(1))
        .and_then(|length| length.parse().ok())
        .ok_or_else(|| anyhow!("Management network {name} has no IPv4 prefix length"))?;
    Ok((prefix, config.gateway))
}

/// The guest config for `spec`: Docker's command and environment rules over the image's own
/// entrypoint, command, environment, user and working directory.
async fn guest_config(
    docker: &Docker,
    spec: &ContainerSpec,
    model_kind: ModelKind,
    node: &MicroVmNode<'_>,
    nics: Vec<MicroVmNic>,
) -> Result<GuestConfig> {
    let image = docker
        .inspect_image(&spec.image)
        .await
        .with_context(|| format!("Cannot inspect image {}", spec.image))?
        .config
        .unwrap_or_default();
    let mut argv = image.entrypoint.unwrap_or_default();
    if spec.commands.is_empty() {
        argv.extend(image.cmd.unwrap_or_default());
    } else {
        argv.extend(spec.commands.iter().cloned());
    }
    let (prefix, gateway) = management_network(docker, node.lab_id).await?;
    Ok(GuestConfig {
        hostname: spec
            .hostname
            .clone()
            .unwrap_or_else(|| node.node_name.to_owned()),
        // SR Linux puts its management address in its own `srbase-mgmt` namespace; the same
        // address in the default namespace would answer for it.
        mgmt_address: spec
            .management_network
            .ipv4_address
            .as_ref()
            .filter(|_| model_kind.model() != NodeModel::NokiaSrlinux)
            .map(|address| format!("{address}/{prefix}")),
        mgmt_gateway: gateway,
        nics,
        argv,
        env: merge_env(image.env.unwrap_or_default(), &spec.env_vars),
        user: spec
            .user
            .clone()
            .or(image.user)
            .filter(|user| !user.is_empty()),
        working_dir: image.working_dir.filter(|dir| !dir.is_empty()),
        volumes: spec
            .volumes
            .iter()
            .filter(|volume| !is_secrets_volume(volume))
            .cloned()
            .collect(),
    })
}

/// Whether `volume` mounts a node's secret files, which reach the guest from RAM rather than
/// in its config ISO.
fn is_secrets_volume(volume: &str) -> bool {
    volume
        .split(':')
        .nth(1)
        .is_some_and(|dst| dst == CONTAINER_SECRETS_DIR)
}

/// The folder of the secrets ISO of node `node_name`, in its lab's secrets folder under `root`.
fn secrets_iso_dir(root: &Path, lab_id: &str, node_name: &str) -> PathBuf {
    root.join(lab_id)
        .join(format!("{node_name}{SECRETS_ISO_DIR_SUFFIX}"))
}

/// Pack the secret files of node `node_name` under `root` into its secrets ISO, in RAM beside
/// them, and return the ISO's path. A node without secret files gets an empty ISO. Written at
/// every boot, so the guest gets the node's files as they are then.
fn write_secrets_iso(root: &Path, lab_id: &str, node_name: &str) -> Result<PathBuf> {
    let secrets = node_secrets_dir(root, lab_id, node_name);
    let volumes: Vec<String> = if secrets.is_dir() {
        vec![format!("{}:{CONTAINER_SECRETS_DIR}:ro", secrets.display())]
    } else {
        Vec::new()
    };
    restrict_dir(root, 0o700, None)?;
    restrict_dir(&root.join(lab_id), 0o700, None)?;
    let dir = secrets_iso_dir(root, lab_id, node_name);
    restrict_dir(&dir, 0o700, None)?;
    let staging = dir.join(SECRETS_STAGING_DIR);
    let iso_path = dir.join(SECRETS_ISO);
    microvm::write_secrets_iso(&volumes, &staging, &iso_path)?;
    fs::remove_dir_all(&staging).with_context(|| format!("Cannot remove {}", staging.display()))?;
    Ok(iso_path)
}

/// Start container node `node` from `spec` in its own microVM: build its image's base disk
/// when there is none, write its config ISO, boot it with a NIC per interface, and put its
/// management tap on the lab's management bridge and each bridge member's tap on its bridge.
/// Returns whether it runs. A microVM that booted but cannot be wired is stopped again.
///
/// # Errors
///
/// Fails when the image, the config or the microVM cannot be made, or a tap cannot be wired.
#[instrument(skip(docker, spec, node, progress), fields(lab_id = %node.lab_id, node_name = %node.node_name))]
pub(crate) async fn start_microvm_node(
    docker: &Docker,
    spec: &ContainerSpec,
    model_kind: ModelKind,
    node: &MicroVmNode<'_>,
    progress: &ProgressSender,
) -> Result<bool> {
    let _ = progress.send_status(
        format!("Node {} - Preparing its microVM disk", node.node_name),
        StatusKind::Progress,
    );
    let base_disk = microvm::ensure_base_disk(docker, &spec.image).await?;
    let nics = node_nics(node, model_kind)?;
    let config = guest_config(docker, spec, model_kind, node, nics.clone()).await?;
    let dir = node_microvm_dir(node.lab_id, node.node_name);
    let config_iso = dir.join(CONFIG_ISO);
    microvm::write_config_iso(&config, &dir.join(CONFIG_STAGING_DIR), &config_iso)?;
    let secrets_iso =
        write_secrets_iso(Path::new(SHERPA_SECRETS_DIR), node.lab_id, node.node_name)?;
    let wiring = tap_wiring(node);
    save_wiring(&dir, &wiring)?;
    console::create_console_dir(node.lab_id)?;
    let serial = console::console_sockets(node.lab_id, node.node_index)?.serial;
    let runtime_dir = microvm::runtime_dir(node.lab_id, node.node_name);
    let vm = MicroVmSpec {
        name: spec.name.clone(),
        runtime_dir: runtime_dir.clone(),
        base_disk,
        config_iso,
        secrets_iso,
        node_disk: dir.join(NODE_DISK),
        node_disk_bytes: NODE_DISK_BYTES,
        serial_socket: PathBuf::from(serial),
        cpus: spec.cpu_count.unwrap_or(DEFAULT_CPUS),
        memory_mib: spec.memory_mib.unwrap_or(DEFAULT_MEMORY_MIB),
        nics,
    };
    microvm::start_microvm(&vm).await?;
    if let Err(error) = wire_taps(&wiring).await {
        let _ = microvm::stop_microvm(&spec.name, &runtime_dir).await;
        return Err(error);
    }
    let _ = progress.send_status(
        format!("Node {} - Started in its microVM", node.node_name),
        StatusKind::Done,
    );
    Ok(microvm::microvm_state(&runtime_dir).await == MicroVmState::Running)
}

/// How the taps of `node` are wired: its management tap on the lab's management bridge, each
/// bridge member's tap on its bridge, and a disabled interface's tap without carrier.
fn tap_wiring(node: &MicroVmNode<'_>) -> TapWiring {
    TapWiring {
        mgmt_tap: mgmt_tap(node.lab_id, node.node_index),
        mgmt_bridge: format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{}", node.lab_id),
        taps: node
            .veths
            .iter()
            .map(|veth| WiredTap {
                tap: veth.host_veth.clone(),
                bridge: veth.bridge.clone(),
                down: veth.admin_down,
            })
            .collect(),
    }
}

/// Wire the taps of a booted microVM as `wiring` says.
async fn wire_taps(wiring: &TapWiring) -> Result<()> {
    network::enslave_to_bridge(&wiring.mgmt_tap, &wiring.mgmt_bridge).await?;
    for tap in &wiring.taps {
        if let Some(bridge) = &tap.bridge {
            network::enslave_to_bridge(&tap.tap, bridge).await?;
        }
        if tap.down {
            network::set_link_state(&tap.tap, false).await?;
        }
    }
    Ok(())
}

/// Keep `wiring` in the node's microVM folder `dir`, for its next boot.
fn save_wiring(dir: &Path, wiring: &TapWiring) -> Result<()> {
    let path = dir.join(TAP_WIRING);
    let text = toml::to_string(wiring).context("Cannot write the microVM's tap wiring")?;
    fs::write(&path, text).with_context(|| format!("Cannot write {}", path.display()))
}

/// The wiring kept in the node's microVM folder `dir`.
fn load_wiring(dir: &Path) -> Result<TapWiring> {
    let path = dir.join(TAP_WIRING);
    let text =
        fs::read_to_string(&path).with_context(|| format!("Cannot read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("Cannot parse {}", path.display()))
}

/// Whether container `container_name` of lab `lab_id` runs in a microVM, and if so the state
/// Docker would report for it.
#[instrument(level = "debug")]
pub(crate) async fn microvm_status(
    lab_id: &str,
    container_name: &str,
) -> Option<ContainerStateStatusEnum> {
    let node_name = container_name.strip_suffix(&format!("-{lab_id}"))?;
    let dir = microvm::runtime_dir(lab_id, node_name);
    if !dir.exists() {
        return None;
    }
    Some(docker_status(microvm::microvm_state(&dir).await))
}

/// The runtime folder of container `container_name` (`<node>-<lab>`) when it runs in a
/// microVM.
pub(crate) fn microvm_dir_of(container_name: &str) -> Option<PathBuf> {
    let (node_name, lab_id) = container_name.rsplit_once('-')?;
    let dir = microvm::runtime_dir(lab_id, node_name);
    dir.exists().then_some(dir)
}

/// The lab of node `node_name` whose container is `container_name` (`<node>-<lab>`), or ""
/// when the container is not the node's.
pub(crate) fn lab_of_container<'a>(container_name: &'a str, node_name: &str) -> &'a str {
    container_name
        .strip_prefix(node_name)
        .and_then(|rest| rest.strip_prefix('-'))
        .unwrap_or_default()
}

/// The Docker state matching microVM state `state`. A guest that stopped itself had its
/// command exit, so its node has failed: `dead`, not `exited`, which a halt by down reports.
fn docker_status(state: MicroVmState) -> ContainerStateStatusEnum {
    match state {
        MicroVmState::Running => ContainerStateStatusEnum::RUNNING,
        MicroVmState::Paused => ContainerStateStatusEnum::PAUSED,
        MicroVmState::Exited => ContainerStateStatusEnum::DEAD,
        MicroVmState::Stopped => ContainerStateStatusEnum::EXITED,
    }
}

/// Every microVM's state as Docker would report its container, keyed by container name.
#[instrument(level = "debug")]
pub(crate) async fn microvm_states() -> HashMap<String, ContainerStateStatusEnum> {
    microvm::list_microvms()
        .await
        .into_iter()
        .map(|(name, state)| (name, docker_status(state)))
        .collect()
}

/// Stop the microVM of node `node_name` in lab `lab_id`, when it has one.
///
/// # Errors
///
/// Fails when the microVM cannot be stopped.
#[instrument(level = "debug")]
pub(crate) async fn stop_microvm_node(lab_id: &str, node_name: &str) -> Result<()> {
    microvm::stop_microvm(
        &format!("{node_name}-{lab_id}"),
        &microvm::runtime_dir(lab_id, node_name),
    )
    .await
    .with_context(|| format!("Cannot stop the microVM of node {node_name}"))
}

/// Halt the microVM of node `node_name` in lab `lab_id`, keeping its disks and definition for
/// `restart_microvm_node`.
///
/// # Errors
///
/// Fails when the microVM cannot be halted.
#[instrument(level = "debug")]
pub(crate) async fn halt_microvm_node(lab_id: &str, node_name: &str) -> Result<()> {
    microvm::halt_microvm(
        &format!("{node_name}-{lab_id}"),
        &microvm::runtime_dir(lab_id, node_name),
    )
    .await
    .with_context(|| format!("Cannot halt the microVM of node {node_name}"))
}

/// Boot the halted microVM of node `node_name` in lab `lab_id` again, with its secrets ISO
/// written again from the node's secret files, and wire its new taps as they were. A microVM
/// whose taps cannot be wired is halted again.
///
/// # Errors
///
/// Fails when the microVM cannot boot or its taps cannot be wired.
#[instrument(level = "debug")]
pub(crate) async fn restart_microvm_node(lab_id: &str, node_name: &str) -> Result<()> {
    let name = format!("{node_name}-{lab_id}");
    let runtime_dir = microvm::runtime_dir(lab_id, node_name);
    let wiring = load_wiring(&node_microvm_dir(lab_id, node_name))?;
    write_secrets_iso(Path::new(SHERPA_SECRETS_DIR), lab_id, node_name)?;
    microvm::restart_microvm(&name, &runtime_dir)
        .await
        .with_context(|| format!("Cannot boot the microVM of node {node_name}"))?;
    if let Err(error) = wire_taps(&wiring).await {
        let _ = microvm::halt_microvm(&name, &runtime_dir).await;
        return Err(error);
    }
    Ok(())
}

/// Resume the paused microVM of node `node_name` in lab `lab_id`.
///
/// # Errors
///
/// Fails when the VMM refuses.
#[instrument(level = "debug")]
pub(crate) async fn resume_microvm_node(lab_id: &str, node_name: &str) -> Result<()> {
    microvm::resume_microvm(&microvm::runtime_dir(lab_id, node_name))
        .await
        .with_context(|| format!("Cannot resume the microVM of node {node_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::node_lifecycle;
    use shared::data::{Halt, Runtime};

    #[test]
    fn privileged_microvm_009_each_nic_of_a_lab_gets_its_own_locally_administered_mac() {
        let macs = [
            nic_mac("abcd1234", 1, 0),
            nic_mac("abcd1234", 1, 1),
            nic_mac("abcd1234", 2, 0),
            nic_mac("abcd1234", 2, 1),
            nic_mac("abcd1234", 300, 0),
        ];
        for (i, mac) in macs.iter().enumerate() {
            assert!(mac.starts_with("02:"), "{mac}");
            assert!(!macs[i + 1..].contains(mac), "{mac} repeats");
        }
    }

    #[test]
    fn privileged_microvm_021_a_microvms_taps_are_wired_at_its_next_boot_as_at_its_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wiring = TapWiring {
            mgmt_tap: "cm1-lab1".to_owned(),
            mgmt_bridge: "brm-lab1".to_owned(),
            taps: vec![
                WiredTap {
                    tap: "tpa0-lab1".to_owned(),
                    bridge: None,
                    down: false,
                },
                WiredTap {
                    tap: "cb1i2-lab1".to_owned(),
                    bridge: Some("brs0-lab1".to_owned()),
                    down: true,
                },
            ],
        };
        save_wiring(dir.path(), &wiring).expect("save");
        assert_eq!(load_wiring(dir.path()).expect("load"), wiring);
    }

    #[test]
    fn privileged_microvm_025_a_microvm_that_stopped_itself_reads_failed_and_a_halted_one_stopped()
    {
        assert_eq!(
            node_lifecycle::container_runtime(Some(docker_status(MicroVmState::Exited))),
            Runtime::Faulted("dead")
        );
        assert_eq!(
            node_lifecycle::container_runtime(Some(docker_status(MicroVmState::Stopped))),
            Runtime::Stopped(Halt::Off)
        );
    }

    #[test]
    fn privileged_microvm_027_a_nodes_secret_files_go_to_a_secrets_iso_beside_them_not_the_config_iso()
     {
        let root = tempfile::tempdir().expect("tempdir");
        let secrets = node_secrets_dir(root.path(), "lab1", "ceos01");
        fs::create_dir_all(&secrets).expect("mkdir");
        fs::write(secrets.join("token"), "s3cret").expect("write");
        let iso = write_secrets_iso(root.path(), "lab1", "ceos01").expect("iso");
        assert_eq!(iso, root.path().join("lab1/ceos01~microvm/secrets.iso"));
        assert!(iso.exists());
        assert!(!root.path().join("lab1/ceos01~microvm/staging").exists());
        assert!(is_secrets_volume(&format!(
            "{}:{CONTAINER_SECRETS_DIR}:ro",
            secrets.display()
        )));
        assert!(!is_secrets_volume(
            "/opt/sherpa/labs/lab1/ceos01/config:/mnt/flash:ro"
        ));
    }

    #[test]
    fn privileged_microvm_010_the_manifest_environment_overrides_the_image_environment() {
        let merged = merge_env(
            vec!["PATH=/bin".to_owned(), "LANG=C".to_owned()],
            &["LANG=C.UTF-8".to_owned(), "CEOS=1".to_owned()],
        );
        assert_eq!(merged, ["PATH=/bin", "LANG=C.UTF-8", "CEOS=1"]);
    }
}
