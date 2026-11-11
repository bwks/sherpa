//! Phase 13: starting the containers and waiting for every node to become ready.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use bollard::secret::ContainerStateStatusEnum;
use tracing::instrument;

use super::context::LabBuild;
use super::links::{P2pAttachment, P2pContainerVeth, attach_ready_p2p_links};
use crate::services::lab_operations::LabOperation;
use crate::services::node_lifecycle;
use crate::services::node_ops::{self, VmReadiness};
use crate::services::node_secrets::write_node_secrets;
use crate::services::talos;

use container::ContainerSpec;
use shared::data;
use shared::data::{NodeState, OperationEvent, StatusKind};
use shared::konst::{
    CONTAINER_SECRETS_DIR, READINESS_SLEEP, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_SECRETS_DIR,
    SSH_PORT,
};
use shared::util;

/// What waiting for the nodes needs besides the build.
pub(super) struct ReadinessInputs<'a> {
    pub(super) container_nodes: &'a [topology::NodeExpanded],
    pub(super) vm_nodes: &'a [topology::NodeExpanded],
    pub(super) unikernel_nodes: &'a [topology::NodeExpanded],
    pub(super) lab_node_data: &'a [data::LabNodeData],
    pub(super) ztp_records: &'a [data::ZtpRecord],
    pub(super) container_volumes: &'a HashMap<String, Vec<String>>,
    pub(super) container_link_networks: &'a HashMap<String, Vec<data::ContainerNetworkAttachment>>,
    pub(super) p2p_container_veths: &'a [P2pContainerVeth],
    pub(super) p2p_links: &'a [&'a data::LabLinkData],
    pub(super) links_detailed: &'a [topology::LinkDetailed],
    pub(super) node_secrets: &'a BTreeMap<String, Vec<data::SecretFile>>,
    pub(super) talos_clusters: &'a [topology::TalosCluster],
    pub(super) timeout: Duration,
}

/// Every node's state once polling ended, and the nodes that failed or were not ready.
pub(super) struct NodesReady {
    pub(super) nodes: Vec<data::NodeInfo>,
    pub(super) errors: Vec<data::UpError>,
}

/// Start the containers and wait for every node to become ready, until `inputs.timeout`
/// (phase 13). A node that is not ready in time, or a container that exits, is a non-critical
/// error: the rest of the lab is kept.
///
/// Every node is `Building` until here. A ready node is recorded `Running`, a node with its
/// check skipped `Running`, a Talos cluster worker or a node not ready in time `Starting` (the
/// scanner records it running once it answers), and a container that exits `Failed`.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn wait_for_nodes(
    build: &LabBuild<'_>,
    operation: &LabOperation,
    inputs: &ReadinessInputs<'_>,
    p2p: &mut P2pAttachment,
) -> Result<NodesReady> {
    let started = Instant::now();
    let all_nodes: Vec<&topology::NodeExpanded> = inputs
        .container_nodes
        .iter()
        .chain(inputs.unikernel_nodes)
        .chain(inputs.vm_nodes)
        .collect();
    let node_names = all_nodes
        .iter()
        .map(|node| node.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let _ = build.progress.send_status(
        format!("Waiting for nodes: {node_names}"),
        StatusKind::Waiting,
    );

    let mut readiness = Readiness {
        build,
        operation,
        inputs,
        p2p,
        secret_mounts: write_secret_mounts(build, inputs.node_secrets)?,
        connected: HashSet::new(),
        nodes: Vec::new(),
        errors: Vec::new(),
    };
    readiness.skip_unpolled_nodes(&all_nodes).await?;
    tracing::info!(
        total_nodes = all_nodes.len(),
        timeout_secs = inputs.timeout.as_secs(),
        "Starting node readiness polling"
    );

    while started.elapsed() < inputs.timeout && readiness.connected.len() < all_nodes.len() {
        for container in inputs.container_nodes {
            if !readiness.connected.contains(&container.name) {
                readiness.start_container(container).await?;
            }
        }
        for vm in inputs.vm_nodes {
            if !readiness.connected.contains(&vm.name) {
                readiness.check_vm(vm).await?;
            }
        }
        for unikernel in inputs.unikernel_nodes {
            if !readiness.connected.contains(&unikernel.name) {
                readiness.check_unikernel(unikernel).await?;
            }
        }
        if readiness.connected.len() < all_nodes.len() {
            operation
                .sleep(Duration::from_secs(READINESS_SLEEP))
                .await?;
        }
    }

    readiness.recheck_containers().await?;
    for link in inputs.p2p_links {
        if !readiness.p2p.attached.contains(&link.index) {
            tracing::warn!(
                tap_a = %link.tap_a,
                tap_b = %link.tap_b,
                "P2p link not attached: one of its containers did not start"
            );
        }
    }
    readiness.report(&all_nodes, started.elapsed()).await?;
    Ok(NodesReady {
        nodes: readiness.nodes,
        errors: readiness.errors,
    })
}

/// Write each container's secret files once, to RAM, and return the mount each is started
/// with, by node name.
fn write_secret_mounts<'a>(
    build: &LabBuild<'_>,
    node_secrets: &'a BTreeMap<String, Vec<data::SecretFile>>,
) -> Result<HashMap<&'a str, String>> {
    let mut mounts = HashMap::new();
    for (node_name, files) in node_secrets {
        let dir = write_node_secrets(
            Path::new(SHERPA_SECRETS_DIR),
            build.lab_id,
            node_name,
            files,
        )
        .with_context(|| format!("Cannot write secret files for node '{node_name}'"))?;
        let _ = build.progress.send_status(
            format!(
                "Wrote {} secret files for container: {node_name}",
                files.len()
            ),
            StatusKind::Progress,
        );
        mounts.insert(
            node_name.as_str(),
            format!("{}:{CONTAINER_SECRETS_DIR}:ro", dir.display()),
        );
    }
    Ok(mounts)
}

/// Readiness polling in progress: which nodes are done, what they report, and what failed.
struct Readiness<'a, 'b> {
    build: &'a LabBuild<'b>,
    operation: &'a LabOperation,
    inputs: &'a ReadinessInputs<'a>,
    p2p: &'a mut P2pAttachment,
    secret_mounts: HashMap<&'a str, String>,
    /// Nodes that need no more polling: ready, skipped or failed.
    connected: HashSet<String>,
    nodes: Vec<data::NodeInfo>,
    errors: Vec<data::UpError>,
}

impl Readiness<'_, '_> {
    /// Mark the nodes that are not polled as done: VMs and unikernels with `skip_ready_check`,
    /// and Talos cluster workers, which are checked as Kubernetes nodes once their cluster is
    /// bootstrapped. Skipped containers are handled when they start.
    async fn skip_unpolled_nodes(&mut self, all_nodes: &[&topology::NodeExpanded]) -> Result<()> {
        for node in all_nodes {
            let cluster_worker =
                talos::readiness_waits_for_kubernetes(self.inputs.talos_clusters, &node.name);
            let is_container = self
                .inputs
                .container_nodes
                .iter()
                .any(|container| container.name == node.name);
            if !skips_readiness_before_polling(
                node.skip_ready_check.unwrap_or(false),
                is_container,
                cluster_worker,
            ) {
                continue;
            }
            tracing::info!(node_name = %node.name, cluster_worker, "Skipping ready check for node");
            let message = if cluster_worker {
                format!(
                    "Node {} - Ready check deferred to its Kubernetes cluster",
                    node.name
                )
            } else {
                format!("Node {} - Ready check skipped", node.name)
            };
            let _ = self.build.progress.send_status(message, StatusKind::Done);
            // A worker is ready once its cluster is; a skipped check counts the node ready.
            let events: &[OperationEvent] = if cluster_worker {
                &[OperationEvent::Launched]
            } else {
                &[OperationEvent::Launched, OperationEvent::Ready]
            };
            let state = self.record(&node.name, events, None).await?;
            self.connected(
                node,
                state,
                node.ipv4_address.map(|ip| ip.to_string()),
                None,
            );
        }
        Ok(())
    }

    /// Record `node` as done, in `state`.
    fn connected(
        &mut self,
        node: &topology::NodeExpanded,
        state: NodeState,
        ip_address: Option<String>,
        ssh_port: Option<u16>,
    ) {
        self.connected.insert(node.name.clone());
        self.nodes.push(data::NodeInfo {
            name: node.name.clone(),
            kind: node.model.kind(),
            model: node.model,
            status: state,
            ip_address,
            ssh_port,
        });
    }

    /// Record `events` for node `name`, in order, with the SSH host key it offered when given,
    /// and return its state after them.
    async fn record(
        &self,
        name: &str,
        events: &[OperationEvent],
        host_key: Option<&str>,
    ) -> Result<NodeState> {
        let node = self
            .inputs
            .lab_node_data
            .iter()
            .find(|node| node.name == name)
            .with_context(|| format!("Node {name} has no record"))?;
        let record_id = db::get_node_id(&node.record)?;
        if host_key.is_some() {
            db::update_node_host_key(self.build.db, record_id.clone(), host_key).await?;
        }
        let mut state = NodeState::Building;
        for event in events {
            state = node_lifecycle::record(self.build.db, self.operation, &record_id, *event)
                .await
                .with_context(|| format!("Cannot record node {name}"))?;
        }
        Ok(state)
    }

    /// Start a container, then create its P2p interfaces in its network namespace and attach
    /// the links that now have both ends. A container counts as ready once it runs.
    async fn start_container(&mut self, container: &topology::NodeExpanded) -> Result<()> {
        let lab_id = self.build.lab_id;
        let mgmt_ipv4 = container.ipv4_address.map(|ip| ip.to_string());
        let container_name = format!("{}-{lab_id}", container.name);
        let image_name = container
            .image
            .as_ref()
            .ok_or_else(|| anyhow!("Container image not set for node: {}", container.name))?;
        let version = container
            .version
            .as_ref()
            .ok_or_else(|| anyhow!("Container version not set for node: {}", container.name))?;
        let mut volumes = self
            .inputs
            .container_volumes
            .get(&container.name)
            .cloned()
            .with_context(|| format!("Container node '{}' has no generated ZTP", container.name))?;
        if let Some(mount) = self.secret_mounts.get(container.name.as_str()) {
            volumes.push(mount.clone());
        }
        let management_network = data::ContainerNetworkAttachment {
            name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{lab_id}"),
            ipv4_address: mgmt_ipv4.clone(),
            ipv6_address: None,
            linux_interface_name: None,
            admin_down: false,
        };
        let data_networks = self
            .inputs
            .container_link_networks
            .get(&container.name)
            .cloned()
            .unwrap_or_default();

        let is_running = node_ops::start_container_node(
            self.build.docker_conn,
            ContainerSpec {
                name: container_name.clone(),
                image: container::image_reference(image_name, version),
                env_vars: container.environment_variables.clone().unwrap_or_default(),
                volumes,
                capabilities: node_ops::model_capabilities(container.model),
                management_network,
                additional_networks: data_networks,
                commands: container.commands.clone().unwrap_or_default(),
                privileged: container.privileged.unwrap_or(false),
                shm_size: container.shm_size,
                user: container.user.clone(),
            },
            container.model,
            self.build.progress,
        )
        .await?;

        if !is_running {
            // A skipped container is started once and not waited for.
            if container.skip_ready_check.unwrap_or(false) {
                tracing::info!(node_name = %container.name, "Container not running; ready check skipped");
                let _ = self.build.progress.send_status(
                    format!("Node {} - Not running, ready check skipped", container.name),
                    StatusKind::Done,
                );
                // The scanner records what it is once it looks.
                let state = self
                    .record(&container.name, &[OperationEvent::Launched], None)
                    .await?;
                self.connected(container, state, mgmt_ipv4, None);
            } else {
                // Starting it again would fail on its name, so the node is reported failed and
                // the rest of the lab carries on.
                tracing::warn!(node_name = %container.name, "Container exited right after starting");
                let _ = self.build.progress.send_status(
                    format!("Node {} - Exited right after starting", container.name),
                    StatusKind::Waiting,
                );
                self.errors
                    .push(exited_container_error(&container.name, &container_name));
                let state = self
                    .record(&container.name, &[OperationEvent::Failed], None)
                    .await?;
                self.connected(container, state, mgmt_ipv4, None);
            }
            return Ok(());
        }

        self.create_p2p_interfaces(container, &container_name)
            .await?;
        let state = self
            .record(
                &container.name,
                &[OperationEvent::Launched, OperationEvent::Ready],
                None,
            )
            .await?;
        self.connected(container, state, mgmt_ipv4, Some(SSH_PORT));
        Ok(())
    }

    /// Create a started P2p container's data interfaces in its network namespace, named and
    /// up, so nothing runs in the image; then attach the links whose ends now both exist.
    async fn create_p2p_interfaces(
        &mut self,
        container: &topology::NodeExpanded,
        container_name: &str,
    ) -> Result<()> {
        let veths: Vec<&P2pContainerVeth> = self
            .inputs
            .p2p_container_veths
            .iter()
            .filter(|veth| veth.node_name == container.name)
            .collect();
        if veths.is_empty() {
            return Ok(());
        }
        let pid = container::get_container_pid(self.build.docker_conn, container_name).await?;
        for veth in veths {
            let target_name = util::container_interface_name(&veth.node_model, veth.interface_idx)?;
            let state = node_ops::netns_veth_state(veth.node_model, veth.admin_down);
            network::create_veth_in_netns(
                &veth.host_veth,
                &veth.host_alias,
                &target_name,
                pid,
                state,
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to create P2p interface {target_name} in container {container_name}"
                )
            })?;
            if let Some(bridge_name) = &veth.bridge {
                network::enslave_to_bridge(&veth.host_veth, bridge_name).await?;
            }
            self.p2p.ready_ends.insert(veth.host_veth.clone());
            tracing::info!(
                container = %container_name,
                host_veth = %veth.host_veth,
                target = %target_name,
                admin_down = veth.admin_down,
                pid,
                "Created P2p veth into container netns"
            );
        }
        attach_ready_p2p_links(
            self.build.lab_id,
            self.inputs.p2p_links,
            self.inputs.links_detailed,
            &self.p2p.ready_ends,
            &mut self.p2p.attached,
        )
        .await?;
        Ok(())
    }

    /// Check a VM's readiness port; once it answers, record the SSH host key it offered.
    async fn check_vm(&mut self, vm: &topology::NodeExpanded) -> Result<()> {
        let Some(record) = self
            .inputs
            .ztp_records
            .iter()
            .find(|record| record.node_name == vm.name)
        else {
            return Ok(());
        };
        let ready_port = node_ops::vm_ready_port(vm.model, vm.ready_port);
        let ready_label = node_ops::ready_port_label(ready_port);
        match node_ops::check_vm_ready(&record.ipv4_address.to_string(), ready_port).await? {
            VmReadiness::Ready { host_key } => {
                tracing::info!(node_name = %vm.name, ipv4 = %record.ipv4_address, ready_port, "VM ready");
                let _ = self.build.progress.send_status(
                    format!("Node {} - Ready ({ready_label} reachable)", vm.name),
                    StatusKind::Done,
                );
                let state = self
                    .record(
                        &vm.name,
                        &[OperationEvent::Launched, OperationEvent::Ready],
                        host_key.as_deref(),
                    )
                    .await?;
                self.connected(
                    vm,
                    state,
                    Some(record.ipv4_address.to_string()),
                    vm.model.has_ssh().then_some(SSH_PORT),
                );
            }
            VmReadiness::NotReady => {
                tracing::debug!(node_name = %vm.name, ready_port, "Waiting for VM readiness port");
                let _ = self.build.progress.send_status(
                    format!("Node {} - Waiting for {ready_label}", vm.name),
                    StatusKind::Waiting,
                );
            }
        }
        Ok(())
    }

    async fn check_unikernel(&mut self, unikernel: &topology::NodeExpanded) -> Result<()> {
        let domain_name = format!("{}-{}", unikernel.name, self.build.lab_id);
        let mgmt_ip = unikernel.ipv4_address.map(|ip| ip.to_string());
        let ready = node_ops::check_unikernel_ready(
            self.build.qemu_conn,
            &domain_name,
            unikernel.ready_port,
            mgmt_ip.as_deref(),
        )?;
        if !ready {
            tracing::debug!(node_name = %unikernel.name, "Waiting for unikernel to become ready");
            let _ = self.build.progress.send_status(
                format!("Node {} - Waiting for unikernel", unikernel.name),
                StatusKind::Waiting,
            );
            return Ok(());
        }
        tracing::info!(node_name = %unikernel.name, "Unikernel ready");
        let _ = self.build.progress.send_status(
            format!("Node {} - Ready (Unikernel running)", unikernel.name),
            StatusKind::Done,
        );
        let state = self
            .record(
                &unikernel.name,
                &[OperationEvent::Launched, OperationEvent::Ready],
                None,
            )
            .await?;
        self.connected(unikernel, state, mgmt_ip, None);
        Ok(())
    }

    /// Check each started container again: a container counts as ready once it starts, so one
    /// that exited since is reported here and recorded failed.
    async fn recheck_containers(&mut self) -> Result<()> {
        for container in self.inputs.container_nodes {
            if container.skip_ready_check.unwrap_or(false) {
                continue;
            }
            let running = self
                .nodes
                .iter()
                .any(|info| info.name == container.name && info.status == NodeState::Running);
            if !running {
                continue;
            }
            let container_name = format!("{}-{}", container.name, self.build.lab_id);
            match container::container_status(self.build.docker_conn, &container_name).await {
                Ok(Some(ContainerStateStatusEnum::RUNNING)) => {
                    tracing::debug!(node_name = %container.name, "Container still running");
                }
                Ok(_) => {
                    tracing::warn!(node_name = %container.name, "Container exited after starting");
                    self.errors
                        .push(exited_container_error(&container.name, &container_name));
                    let state = self
                        .record(&container.name, &[OperationEvent::Failed], None)
                        .await?;
                    if let Some(info) = self
                        .nodes
                        .iter_mut()
                        .find(|info| info.name == container.name)
                    {
                        info.status = state;
                    }
                }
                Err(error) => tracing::warn!(
                    node_name = %container.name,
                    %error,
                    "Cannot check whether the container is still running"
                ),
            }
        }
        Ok(())
    }

    /// Report how polling ended, with an error for each node that did not become ready. Those
    /// are recorded `Starting`: their domains run, and the scanner records them running once
    /// they answer.
    async fn report(
        &mut self,
        all_nodes: &[&topology::NodeExpanded],
        elapsed: Duration,
    ) -> Result<()> {
        let ready = self.connected.len();
        let total = all_nodes.len();
        if ready == total {
            tracing::info!(
                nodes_ready = ready,
                duration_secs = elapsed.as_secs(),
                "All nodes ready"
            );
            let _ = self
                .build
                .progress
                .send_status("All nodes are ready!".to_string(), StatusKind::Done);
            return Ok(());
        }
        tracing::warn!(
            nodes_ready = ready,
            total_nodes = total,
            timeout_secs = self.inputs.timeout.as_secs(),
            "Timeout reached - not all nodes ready"
        );
        let _ = self.build.progress.send_status(
            format!("Timeout reached. {ready} of {total} nodes are ready."),
            StatusKind::Waiting,
        );
        for node in all_nodes {
            if !self.connected.contains(&node.name) {
                tracing::warn!(node_name = %node.name, "Node not ready after timeout");
                self.record(&node.name, &[OperationEvent::Launched], None)
                    .await?;
                self.errors.push(not_ready_error(&node.name));
            }
        }
        Ok(())
    }
}

/// Whether a node counts as ready before readiness polling starts: one with
/// `skip_ready_check`, or a Talos cluster worker, which is checked as a Kubernetes node later.
/// Containers are started during polling, so a skipped container is handled when it starts.
fn skips_readiness_before_polling(
    skip_ready_check: bool,
    is_container: bool,
    cluster_worker: bool,
) -> bool {
    !is_container && (skip_ready_check || cluster_worker)
}

/// The warning for a node not ready when the wait timed out. It is left `Starting`, and the
/// scanner records it running once it answers, so the lab is still created.
fn not_ready_error(node_name: &str) -> data::UpError {
    data::UpError {
        phase: "NodeReadiness".to_string(),
        message: format!("Node {node_name} did not become ready"),
        is_critical: false,
    }
}

/// The critical error for a container node whose process exited after it started. The node is
/// `Failed` and stays so until it is redeployed, so the lab is only partly created.
fn exited_container_error(node_name: &str, container_name: &str) -> data::UpError {
    data::UpError {
        phase: "NodeReadiness".to_string(),
        message: format!(
            "Node {node_name} exited right after starting; see `docker logs {container_name}` on the server"
        ),
        is_critical: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_up_with_a_node_that_will_not_recover_did_not_succeed() {
        // M6.1: every error used to be a warning, so `up` reported success for a lab with a
        // container that exited. A node that is only slow to answer still is a warning.
        assert!(exited_container_error("r1", "r1-ab12cd34").is_critical);
        assert!(!not_ready_error("r1").is_critical);
    }

    #[test]
    fn test_skipped_vm_is_ready_before_polling() {
        assert!(skips_readiness_before_polling(true, false, false));
    }

    #[test]
    fn test_talos_worker_is_ready_before_polling() {
        assert!(skips_readiness_before_polling(false, false, true));
    }

    #[test]
    fn test_skipped_container_waits_for_polling_to_start_it() {
        // Containers are started during polling, so skipping their ready check there would
        // skip starting them.
        assert!(!skips_readiness_before_polling(true, true, false));
    }

    #[test]
    fn test_node_without_skip_is_polled() {
        assert!(!skips_readiness_before_polling(false, false, false));
        assert!(!skips_readiness_before_polling(false, true, false));
    }
}
