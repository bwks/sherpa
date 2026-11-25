//! Container nodes whose model runs in a Cloud Hypervisor microVM: the container's spec,
//! turned into a microVM with the same image, command, environment, mounts and interfaces.
//! See `docs/architecture/MICROVM.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use bollard::Docker;
use bollard::models::ContainerStateStatusEnum;
use bollard::query_parameters::InspectNetworkOptions;
use container::ContainerSpec;
use container::microvm::{self, GuestConfig, MicroVmNic, MicroVmSpec, MicroVmState};
use tracing::instrument;

use crate::konst::{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX, SHERPA_MANAGEMENT_NETWORK_NAME};
use crate::services::console;
use crate::services::p2p_links::ContainerVeth;
use crate::services::progress::ProgressSender;
use shared::data::{ModelKind, NodeModel, StatusKind};
use shared::konst::SHERPA_LABS_PATH;
use shared::util;

const MICROVM_NODE_DIR: &str = "microvm";
const CONFIG_STAGING_DIR: &str = "config";
const CONFIG_ISO: &str = "config.iso";
const NODE_DISK: &str = "node.raw";
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
        volumes: spec.volumes.clone(),
    })
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
    console::create_console_dir(node.lab_id)?;
    let serial = console::console_sockets(node.lab_id, node.node_index)?.serial;
    let runtime_dir = microvm::runtime_dir(node.lab_id, node.node_name);
    let vm = MicroVmSpec {
        name: spec.name.clone(),
        runtime_dir: runtime_dir.clone(),
        base_disk,
        config_iso,
        node_disk: dir.join(NODE_DISK),
        node_disk_bytes: NODE_DISK_BYTES,
        serial_socket: PathBuf::from(serial),
        cpus: spec.cpu_count.unwrap_or(DEFAULT_CPUS),
        memory_mib: spec.memory_mib.unwrap_or(DEFAULT_MEMORY_MIB),
        nics,
    };
    microvm::start_microvm(&vm).await?;
    if let Err(error) = wire_taps(node).await {
        let _ = microvm::stop_microvm(&spec.name, &runtime_dir).await;
        return Err(error);
    }
    let _ = progress.send_status(
        format!("Node {} - Started in its microVM", node.node_name),
        StatusKind::Done,
    );
    Ok(microvm::microvm_state(&runtime_dir).await == MicroVmState::Running)
}

/// Put the management tap of `node` on the lab's management bridge, each bridge member's tap
/// on its bridge, and take the carrier from a disabled interface.
async fn wire_taps(node: &MicroVmNode<'_>) -> Result<()> {
    let bridge = format!("{SHERPA_MANAGEMENT_NETWORK_BRIDGE_PREFIX}-{}", node.lab_id);
    network::enslave_to_bridge(&mgmt_tap(node.lab_id, node.node_index), &bridge).await?;
    for veth in &node.veths {
        if let Some(bridge) = &veth.bridge {
            network::enslave_to_bridge(&veth.host_veth, bridge).await?;
        }
        if veth.admin_down {
            network::set_link_state(&veth.host_veth, false).await?;
        }
    }
    Ok(())
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

/// The Docker state matching microVM state `state`.
fn docker_status(state: MicroVmState) -> ContainerStateStatusEnum {
    match state {
        MicroVmState::Running => ContainerStateStatusEnum::RUNNING,
        MicroVmState::Paused => ContainerStateStatusEnum::PAUSED,
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn privileged_microvm_010_the_manifest_environment_overrides_the_image_environment() {
        let merged = merge_env(
            vec!["PATH=/bin".to_owned(), "LANG=C".to_owned()],
            &["LANG=C.UTF-8".to_owned(), "CEOS=1".to_owned()],
        );
        assert_eq!(merged, ["PATH=/bin", "LANG=C.UTF-8", "CEOS=1"]);
    }
}
