//! Phase 13: starting the containers and waiting for every node to become ready.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use bollard::secret::ContainerStateStatusEnum;
use tracing::instrument;

use super::context::LabBuild;
use super::links::{P2pAttachment, P2pContainerVeth, attach_ready_p2p_links};
use crate::konst::{READINESS_SLEEP, SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_SECRETS_DIR};
use crate::services::lab_operations::LabOperation;
use crate::services::node_lifecycle;
use crate::services::node_ops::{self, VmReadiness};
use crate::services::node_secrets::write_node_secrets;
use crate::services::p2p_links::{KernelHost, LinkHost};
use crate::services::talos;

use container::ContainerSpec;
use shared::data;
use shared::data::{Halt, NodeState, OperationEvent, Runtime, StatusKind};
use shared::konst::{CONTAINER_SECRETS_DIR, SSH_PORT};

/// How long a started container must keep running before it counts as ready, or before its
/// `ready_check.port` is waited for. An image whose entrypoint fails, such as mysql without a
/// root password, exits a moment after Docker reports it running.
const CONTAINER_SETTLE: Duration = Duration::from_secs(5);
/// The shortest wait between two polls, so a container between states is not polled in a loop.
const MIN_POLL_INTERVAL: Duration = Duration::from_secs(1);
/// How many of its last log lines a container that exited reports.
const EXIT_LOG_LINES: u32 = 10;

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
/// (phase 13). A node that is not ready in time is a non-critical error, and a container that
/// exits a critical one; either way the rest of the lab is kept.
///
/// Every node is `Building` until here. A started container is recorded `Starting`, and
/// `Running` once it has kept running for `CONTAINER_SETTLE` and, with a `ready_check.port`,
/// that port answers on its management address. A ready node is recorded
/// `Running`, a node with its check skipped `Running`, a Talos cluster worker or a node not
/// ready in time `Starting` (the scanner records it running once it answers), and a container
/// that exits, at start, while it settles or by the end of polling, `Failed`.
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
        settling: HashMap::new(),
        waiting_for_port: HashSet::new(),
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
            if readiness.connected.contains(&container.name) {
                continue;
            }
            match readiness.settling.get(&container.name) {
                Some(started) => readiness.check_container(container, *started).await?,
                None => readiness.start_container(container).await?,
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
            let interval = poll_interval(
                readiness
                    .settling
                    .iter()
                    .filter(|(name, _)| {
                        !readiness.connected.contains(*name)
                            && !readiness.waiting_for_port.contains(*name)
                    })
                    .map(|(_, started)| started.elapsed()),
            );
            operation.sleep(interval).await?;
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
    /// Containers started and waited for, by node name, with when each was started.
    settling: HashMap<String, Instant>,
    /// Settled containers waiting for their ready port, polled as VMs are rather than as
    /// settling containers.
    waiting_for_port: HashSet<String>,
    nodes: Vec<data::NodeInfo>,
    errors: Vec<data::UpError>,
}

impl Readiness<'_, '_> {
    /// Mark the nodes that are not polled as done: VMs and unikernels with `ready_check.skip`,
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
                node.ready_check.skip.unwrap_or(false),
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
            kind: node.model_kind.kind(),
            model: node.model_kind.model(),
            status: state,
            ip_address,
            ssh_port,
        });
    }

    /// The data of node `name`, with its record.
    fn node_data(&self, name: &str) -> Result<&data::LabNodeData> {
        self.inputs
            .lab_node_data
            .iter()
            .find(|node| node.name == name)
            .with_context(|| format!("Node {name} has no record"))
    }

    /// Record `events` for node `name`, in order, with the SSH host key it offered when given,
    /// and return its state after them.
    async fn record(
        &self,
        name: &str,
        events: &[OperationEvent],
        host_key: Option<&str>,
    ) -> Result<NodeState> {
        let node = self.node_data(name)?;
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
    /// the links that now have both ends. It is then waited for in `check_container`, unless
    /// its check is skipped.
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
        let settings = container.container.clone().unwrap_or_default();
        // Its limits are the resources recorded for it.
        let record = &self.node_data(&container.name)?.record;

        let is_running = node_ops::start_container_node(
            self.build.docker_conn,
            self.build.qemu,
            ContainerSpec {
                name: container_name.clone(),
                // The node name, as its own config names it; Docker's default is the ID.
                hostname: Some(container.name.clone()),
                image: container::image_reference(image_name, version),
                env_vars: container.environment_variables.clone().unwrap_or_default(),
                volumes,
                capabilities: node_ops::model_capabilities(container.model_kind),
                management_network,
                additional_networks: data_networks,
                commands: settings.commands.unwrap_or_default(),
                privileged: settings.privileged.unwrap_or(false),
                shm_size: settings.shm_size,
                user: settings.user,
                memory_mib: record.memory.map(u64::from),
                cpu_count: record.cpu_count,
            },
            container.model_kind,
            self.build.progress,
        )
        .await?;

        if !is_running {
            tracing::warn!(node_name = %container.name, "Container exited right after starting");
            // Starting it again would fail on its name, so the node is reported failed and the
            // rest of the lab carries on.
            let state = self
                .fail_exited_container(container, &container_name)
                .await?;
            self.connected(container, state, mgmt_ipv4, None);
            return Ok(());
        }

        if let Err(error) = self.create_p2p_interfaces(container, &container_name).await {
            // Its namespace goes with its process, so a container that exited meanwhile fails
            // here; that is the container's failure, not the lab's.
            if self.is_running(&container_name).await {
                return Err(error);
            }
            tracing::warn!(node_name = %container.name, error = %error, "Container exited while its P2p interfaces were created");
            let state = self
                .fail_exited_container(container, &container_name)
                .await?;
            self.connected(container, state, mgmt_ipv4, None);
            return Ok(());
        }

        // A skipped container is started once and not waited for.
        if container.ready_check.skip.unwrap_or(false) {
            tracing::info!(node_name = %container.name, "Container started; ready check skipped");
            let _ = self.build.progress.send_status(
                format!("Node {} - Ready check skipped", container.name),
                StatusKind::Done,
            );
            let state = self
                .record(
                    &container.name,
                    &[OperationEvent::Launched, OperationEvent::Ready],
                    None,
                )
                .await?;
            self.connected(container, state, mgmt_ipv4, Some(SSH_PORT));
            return Ok(());
        }

        self.record(&container.name, &[OperationEvent::Launched], None)
            .await?;
        self.settling.insert(container.name.clone(), Instant::now());
        Ok(())
    }

    /// Check a container started `started`: ready once it has kept running for
    /// `CONTAINER_SETTLE` and its ready port, when it has one, answers on its management
    /// address; failed when it stopped running before. The port is checked as a VM's is.
    async fn check_container(
        &mut self,
        container: &topology::NodeExpanded,
        started: Instant,
    ) -> Result<()> {
        let container_name = format!("{}-{}", container.name, self.build.lab_id);
        let ready_port =
            node_ops::container_ready_port(container.model_kind, &container.ready_check);
        let ready_label = ready_port.map(node_ops::ready_port_label);
        let status = match container::container_state(self.build.docker_conn, &container_name).await
        {
            Ok(state) => state.and_then(|state| state.status),
            Err(error) => {
                // Docker did not answer; the next poll asks again, and the readiness timeout
                // ends the wait.
                tracing::warn!(node_name = %container.name, %error, "Cannot check the container");
                return Ok(());
            }
        };
        let mgmt_ipv4 = container.ipv4_address.map(|ip| ip.to_string());
        let port_check = match (ready_port, mgmt_ipv4.as_deref()) {
            (Some(port), Some(ip)) => Some(node_ops::check_vm_ready(ip, port, false).await?),
            (Some(_), None) => Some(VmReadiness::NotReady),
            (None, _) => None,
        };
        match check_started_container(status, started.elapsed(), port_check) {
            ContainerCheck::Settling => {
                tracing::debug!(node_name = %container.name, ?status, "Waiting for the container to settle");
                let _ = self.build.progress.send_status(
                    format!(
                        "Node {} - Waiting for the container to keep running",
                        container.name
                    ),
                    StatusKind::Waiting,
                );
            }
            ContainerCheck::WaitingForPort => {
                tracing::debug!(node_name = %container.name, ?ready_port, "Waiting for container readiness port");
                let _ = self.build.progress.send_status(
                    format!(
                        "Node {} - Waiting for {}",
                        container.name,
                        ready_label.as_deref().unwrap_or("its ready port")
                    ),
                    StatusKind::Waiting,
                );
                self.waiting_for_port.insert(container.name.clone());
            }
            ContainerCheck::Ready { host_key } => {
                tracing::info!(node_name = %container.name, ?ready_port, "Container ready");
                let reason = match &ready_label {
                    Some(label) => format!("{label} reachable"),
                    None => "container running".to_string(),
                };
                let _ = self.build.progress.send_status(
                    format!("Node {} - Ready ({reason})", container.name),
                    StatusKind::Done,
                );
                let state = self
                    .record(
                        &container.name,
                        &[OperationEvent::Ready],
                        host_key.as_deref(),
                    )
                    .await?;
                self.connected(container, state, mgmt_ipv4, Some(SSH_PORT));
            }
            ContainerCheck::Exited => {
                tracing::warn!(node_name = %container.name, ?status, "Container stopped running before it was ready");
                let state = self
                    .fail_exited_container(container, &container_name)
                    .await?;
                self.connected(container, state, mgmt_ipv4, None);
            }
        }
        Ok(())
    }

    /// Whether Docker reports the container `container_name` running. A container Docker cannot
    /// inspect is not.
    async fn is_running(&self, container_name: &str) -> bool {
        matches!(
            container::container_status(self.build.docker_conn, container_name).await,
            Ok(Some(ContainerStateStatusEnum::RUNNING))
        )
    }

    /// Report a container node that stopped running, with its exit code and last log lines,
    /// and record it `Failed`. Returns its new state.
    async fn fail_exited_container(
        &mut self,
        container: &topology::NodeExpanded,
        container_name: &str,
    ) -> Result<NodeState> {
        let docker = self.build.docker_conn;
        let exit_code = match container::container_state(docker, container_name).await {
            Ok(state) => state.and_then(|state| state.exit_code),
            Err(error) => {
                tracing::warn!(node_name = %container.name, %error, "Cannot read the container's exit code");
                None
            }
        };
        let log_tail = match container::container_log_tail(docker, container_name, EXIT_LOG_LINES)
            .await
        {
            Ok(log) => Some(log),
            Err(error) => {
                tracing::warn!(node_name = %container.name, %error, "Cannot read the container's log");
                None
            }
        };
        let exit = ContainerExit {
            exit_code,
            log_tail,
        };
        tracing::warn!(node_name = %container.name, ?exit_code, "Container stopped running");
        let _ = self.build.progress.send_status(
            format!("Node {} - {}", container.name, exit.summary()),
            StatusKind::Waiting,
        );
        self.errors.push(exited_container_error(
            &container.name,
            container_name,
            &exit,
        ));
        self.record(&container.name, &[OperationEvent::Failed], None)
            .await
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
        let host = KernelHost::new(self.build.docker_conn, self.build.qemu);
        let pid = host.container_pid(container_name).await?;
        for P2pContainerVeth { veth, .. } in veths {
            host.create_veth(veth, pid).await.with_context(|| {
                format!("Failed to create a P2p interface in container {container_name}")
            })?;
            self.p2p.ready_ends.insert(veth.host_veth.clone());
        }
        attach_ready_p2p_links(
            &host,
            self.build.lab_id,
            self.inputs.p2p_links,
            self.inputs.links_detailed,
            &self.p2p.ready_ends,
            &mut self.p2p.attached,
        )
        .await?;
        Ok(())
    }

    /// Check a VM's readiness port; once it answers, and a Windows node's SSH server runs with
    /// Sherpa's config, record the SSH host key it offered.
    async fn check_vm(&mut self, vm: &topology::NodeExpanded) -> Result<()> {
        let Some(record) = self
            .inputs
            .ztp_records
            .iter()
            .find(|record| record.node_name == vm.name)
        else {
            return Ok(());
        };
        // The check stored for the node: the manifest's port, else the model's own, SSH or the
        // Talos API for Talos Linux; and, for a Windows node, whose SSH server answers before its
        // owner can log in, Sherpa's ready marker.
        let ready_check = node_ops::stored_ready_check(vm.model_kind, &vm.ready_check, record.ztp);
        let ready_port = ready_check
            .port
            .with_context(|| format!("VM '{}' has no readiness port", vm.name))?;
        let ready_label = node_ops::ready_port_label(ready_port);
        match node_ops::check_vm_ready(
            &record.ipv4_address.to_string(),
            ready_port,
            ready_check.sshd_marker,
        )
        .await?
        {
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
                    vm.model_kind.has_ssh().then_some(SSH_PORT),
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
            unikernel.ready_check.port,
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

    /// Check each ready container again, as VMs may have kept polling going long after it
    /// settled: one that exited since is reported here and recorded failed.
    async fn recheck_containers(&mut self) -> Result<()> {
        for container in self.inputs.container_nodes {
            if container.ready_check.skip.unwrap_or(false) {
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
                    let state = self
                        .fail_exited_container(container, &container_name)
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
/// `ready_check.skip`, or a Talos cluster worker, which is checked as a Kubernetes node later.
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

/// How a container that stopped running ended, as far as Docker could tell.
struct ContainerExit {
    exit_code: Option<i64>,
    /// Its last log lines; `None` when they could not be read.
    log_tail: Option<String>,
}

impl ContainerExit {
    /// One line on how it ended, without the log.
    fn summary(&self) -> String {
        match self.exit_code {
            Some(code) => format!("exited with code {code} before it was ready"),
            None => "stopped running before it was ready".to_string(),
        }
    }
}

/// Whether a container started and waited for is ready, from Docker's status for it, how long
/// ago it was started and what its ready port answered.
#[derive(Debug, PartialEq, Eq)]
enum ContainerCheck {
    /// It has kept running for `CONTAINER_SETTLE`, and its ready port, if it has one, answers;
    /// with the host key it offered when that port is SSH.
    Ready { host_key: Option<String> },
    /// It runs, but not for long enough yet, or it is between states.
    Settling,
    /// It has kept running, but its ready port does not answer yet.
    WaitingForPort,
    /// It stopped running, or is gone.
    Exited,
}

/// Decide whether a started container is ready: running for `CONTAINER_SETTLE`, from Docker's
/// `status` and the time `running_for` since it was started, and then, when it has a ready
/// port, answering on it, from `port_check`; `None` when it has no ready port.
fn check_started_container(
    status: Option<ContainerStateStatusEnum>,
    running_for: Duration,
    port_check: Option<VmReadiness>,
) -> ContainerCheck {
    match node_lifecycle::container_runtime(status) {
        Runtime::Running if running_for >= CONTAINER_SETTLE => match port_check {
            None => ContainerCheck::Ready { host_key: None },
            Some(VmReadiness::Ready { host_key }) => ContainerCheck::Ready { host_key },
            Some(VmReadiness::NotReady) => ContainerCheck::WaitingForPort,
        },
        Runtime::Running | Runtime::Transitional(_) | Runtime::Stopped(Halt::Paused) => {
            ContainerCheck::Settling
        }
        Runtime::Stopped(Halt::Off) | Runtime::Faulted(_) | Runtime::Absent => {
            ContainerCheck::Exited
        }
    }
}

/// How long to wait before the next poll: until the first settling container is due, given how
/// long each has run, but never longer than `READINESS_SLEEP` or shorter than
/// `MIN_POLL_INTERVAL`.
fn poll_interval(settling_for: impl Iterator<Item = Duration>) -> Duration {
    settling_for
        .map(|running_for| CONTAINER_SETTLE.saturating_sub(running_for))
        .min()
        .unwrap_or(Duration::from_secs(READINESS_SLEEP))
        .clamp(MIN_POLL_INTERVAL, Duration::from_secs(READINESS_SLEEP))
}

/// The critical error for a container node whose process stopped running before it was ready,
/// with its exit code and last log lines. The node is `Failed` and stays so until it is
/// redeployed, so the lab is only partly created.
fn exited_container_error(
    node_name: &str,
    container_name: &str,
    exit: &ContainerExit,
) -> data::UpError {
    let mut message = format!(
        "Node {node_name} {}; see `docker logs {container_name}` on the server",
        exit.summary()
    );
    if let Some(log) = exit.log_tail.as_deref().filter(|log| !log.is_empty()) {
        message.push_str(". Its last log lines:\n");
        message.push_str(log);
    }
    data::UpError {
        phase: "NodeReadiness".to_string(),
        message,
        is_critical: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exit(exit_code: Option<i64>, log_tail: Option<&str>) -> ContainerExit {
        ContainerExit {
            exit_code,
            log_tail: log_tail.map(str::to_string),
        }
    }

    #[test]
    fn node_failure_005_an_exited_container_error_without_a_code_or_log_says_where_to_look() {
        let error = exited_container_error("db01", "db01-ab12cd34", &exit(None, Some("")));
        assert_eq!(
            error.message,
            "Node db01 stopped running before it was ready; see `docker logs db01-ab12cd34` on \
             the server"
        );
    }

    #[test]
    fn node_ready_010_a_container_is_ready_once_it_has_run_for_5_seconds() {
        let running = Some(ContainerStateStatusEnum::RUNNING);
        assert_eq!(
            check_started_container(running, Duration::ZERO, None),
            ContainerCheck::Settling
        );
        assert_eq!(
            check_started_container(
                running,
                CONTAINER_SETTLE.saturating_sub(Duration::from_millis(1)),
                None
            ),
            ContainerCheck::Settling
        );
        assert_eq!(
            check_started_container(running, CONTAINER_SETTLE, None),
            ContainerCheck::Ready { host_key: None }
        );
    }

    #[test]
    fn node_failure_001_a_container_found_exited_dead_or_never_started_has_failed() {
        // The mysql image exits about a second after it starts without a root password.
        for status in [
            Some(ContainerStateStatusEnum::EXITED),
            Some(ContainerStateStatusEnum::DEAD),
            Some(ContainerStateStatusEnum::CREATED),
            None,
        ] {
            assert_eq!(
                check_started_container(status, Duration::ZERO, None),
                ContainerCheck::Exited,
                "{status:?}"
            );
            assert_eq!(
                check_started_container(status, CONTAINER_SETTLE * 10, None),
                ContainerCheck::Exited,
                "{status:?}"
            );
        }
    }

    #[test]
    fn node_ready_014_a_container_docker_reports_between_states_is_waited_for() {
        for status in [
            ContainerStateStatusEnum::RESTARTING,
            ContainerStateStatusEnum::REMOVING,
            ContainerStateStatusEnum::PAUSED,
            ContainerStateStatusEnum::EMPTY,
        ] {
            assert_eq!(
                check_started_container(Some(status), CONTAINER_SETTLE * 10, None),
                ContainerCheck::Settling,
                "{status:?}"
            );
        }
    }

    fn ready(host_key: Option<&str>) -> VmReadiness {
        VmReadiness::Ready {
            host_key: host_key.map(str::to_string),
        }
    }

    #[test]
    fn node_ready_011_a_container_whose_ready_port_answers_still_waits_its_5_seconds() {
        let running = Some(ContainerStateStatusEnum::RUNNING);
        assert_eq!(
            check_started_container(running, Duration::ZERO, Some(ready(None))),
            ContainerCheck::Settling
        );
    }

    #[test]
    fn node_ready_012_a_settled_container_whose_ready_port_does_not_answer_is_waited_for() {
        let running = Some(ContainerStateStatusEnum::RUNNING);
        assert_eq!(
            check_started_container(running, CONTAINER_SETTLE, Some(VmReadiness::NotReady)),
            ContainerCheck::WaitingForPort
        );
    }

    #[test]
    fn node_ready_013_a_settled_container_whose_ready_port_answers_is_ready() {
        let running = Some(ContainerStateStatusEnum::RUNNING);
        assert_eq!(
            check_started_container(running, CONTAINER_SETTLE, Some(ready(None))),
            ContainerCheck::Ready { host_key: None }
        );
    }

    /// Left for the known hosts spec.
    #[test]
    fn node_host_keys_012_a_container_ready_on_port_22_has_its_key_recorded() {
        let running = Some(ContainerStateStatusEnum::RUNNING);
        // On the SSH port the key it offered is kept, as for a VM.
        assert_eq!(
            check_started_container(
                running,
                CONTAINER_SETTLE,
                Some(ready(Some("ssh-ed25519 AAAA")))
            ),
            ContainerCheck::Ready {
                host_key: Some("ssh-ed25519 AAAA".to_string())
            }
        );
    }

    #[test]
    fn node_failure_002_a_container_that_exits_while_its_ready_port_is_waited_for_has_failed() {
        for status in [Some(ContainerStateStatusEnum::EXITED), None] {
            assert_eq!(
                check_started_container(status, CONTAINER_SETTLE * 10, Some(VmReadiness::NotReady)),
                ContainerCheck::Exited,
                "{status:?}"
            );
        }
    }

    #[test]
    fn node_ready_016_a_vm_with_a_skipped_check_is_ready_before_polling() {
        assert!(skips_readiness_before_polling(true, false, false));
    }

    #[test]
    fn test_talos_worker_is_ready_before_polling() {
        assert!(skips_readiness_before_polling(false, false, true));
    }

    #[test]
    fn node_ready_017_a_container_with_a_skipped_check_waits_for_polling_to_start_it() {
        // Containers are started during polling, so skipping their ready check there would
        // skip starting them.
        assert!(!skips_readiness_before_polling(true, true, false));
    }
}
