//! The `up_lab` orchestrator: setup (phase 1), each phase in turn, and rolling back a lab
//! whose `up` failed.

use std::collections::{BTreeMap, HashMap};
use std::mem;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use opentelemetry::KeyValue;
use tracing::instrument;

use super::context::LabBuild;
use super::links::{LinkRecords, P2pAttachment, attach_vm_links, create_links};
use super::networks::{
    container_data_attachments, create_isolated_container_networks, create_management_networks,
    create_shared_bridges, management_network, set_isolated_bridges_down, write_lab_files,
};
use super::plan::{LabPlan, plan_lab};
use super::readiness::{NodesReady, ReadinessInputs, wait_for_nodes};
use super::records::{LabOwner, NodeRecords, create_node_records};
use super::runtime::{create_unikernels, create_vms};
use super::services::{LabServices, ServiceInputs, start_lab_services};
use super::ssh::write_ssh_files;
use super::ztp::{ConfigInputs, NodeConfigs, configure_nodes, start_boot_container};
use crate::auth::context::AuthContext;
use crate::auth::policy::{require_host_access, require_lab_starter};
use crate::daemon::state::AppState;
use crate::services::clean;
use crate::services::guest_account;
use crate::services::lab_operations::{self, LabOperation, StopReason};
use crate::services::lab_ssh_key::{LabSshKeyPaths, create_lab_ssh_key};
use crate::services::node_ops;
use crate::services::progress::ProgressSender;
use crate::services::ssh_config::{self, LabSshFiles};
use crate::services::tailscale;
use crate::services::talos::LabCluster;
use crate::tls::lab_ca::LabCa;
use crate::validate::{reject_provisioners, validate_lab_origin, validate_node_secrets};

use libvirt::QemuConnection;
use shared::data;
use shared::data::{LabSubnets, RecordId, StatusKind, TailnetAuthKey, TailnetStatus};
use shared::konst::{READINESS_TIMEOUT, SHERPA_LABS_PATH};
use shared::util;
use topology::validate::{check_client_read_files, check_lab_description};

/// What setup (phase 1) gives the later phases: the libvirt connection, the owner's record,
/// and every node image.
struct Setup {
    qemu_conn: Arc<QemuConnection>,
    db_user: data::DbUser,
    node_images: Vec<data::NodeConfig>,
}

/// One `up`: the lab being built, what phases 1 and 2 decided, and the phases completed and
/// non-critical errors so far. The resource-creating phases (3 to 13) are its methods.
struct LabUp<'a> {
    build: LabBuild<'a>,
    state: &'a AppState,
    operation: &'a LabOperation,
    manifest: &'a topology::Manifest,
    owner: LabOwner<'a>,
    /// The authenticated caller, who owns the lab.
    username: &'a str,
    /// Holds the disk image leases until every domain exists.
    plan: LabPlan,
    node_secrets: &'a BTreeMap<String, Vec<data::SecretFile>>,
    tailscale_auth_key: Option<&'a TailnetAuthKey>,
    tailnet_enabled: bool,
    start_time: Instant,
    phases_completed: Vec<String>,
    errors: Vec<data::UpError>,
}

/// The lab's management network and what was made on it, and the lab's SSH key with the
/// sherpa account that logs in with it (phase 4).
struct LabNetwork {
    lab_info: data::LabInfo,
    ssh_key: LabSshKeyPaths,
    sherpa_user: data::User,
    mgmt_net: data::SherpaNetwork,
    dns: data::Dns,
    tailnet_status: Option<TailnetStatus>,
}

/// What creating the nodes (phases 8 to 11) leaves for readiness and the response.
struct BuiltNodes {
    lab_ca: LabCa,
    ztp_records: Vec<data::ZtpRecord>,
    container_volumes: HashMap<String, Vec<String>>,
    lab_clusters: Vec<LabCluster>,
    p2p: P2pAttachment,
}

/// What phases 12 and 13 leave for the response.
struct StartedNodes {
    nodes: Vec<data::NodeInfo>,
    ssh_files: LabSshFiles,
    ssh_private_key: String,
    clusters: Vec<data::ClusterInfo>,
    keycloaks: Vec<data::KeycloakInfo>,
    vaults: Vec<data::VaultInfo>,
}

/// Start a lab with streaming progress updates
///
/// The lab is created for the authenticated caller. `request.username` is ignored. The built-in
/// `admin` account cannot start labs.
#[instrument(skip(ctx, request, state, progress), fields(username = %ctx.username, lab_id = %request.lab_id))]
pub async fn up_lab(
    ctx: &AuthContext,
    request: data::UpRequest,
    state: &AppState,
    progress: ProgressSender,
) -> Result<data::UpResponse> {
    let start_time = Instant::now();
    let data::UpRequest {
        tailscale_auth_key,
        lab_id,
        manifest,
        node_secrets,
        client_host,
        client_folder,
    } = request;
    require_lab_starter(ctx)?;
    let manifest = check_up_request(
        ctx,
        manifest,
        &node_secrets,
        client_host.as_deref(),
        client_folder.as_deref(),
    )?;
    let details = db::LabDetails {
        description: manifest.description.clone(),
        client_host,
        client_folder,
    };

    let tailnet_enabled = tailscale::validate_enrollment(
        &manifest,
        tailscale_auth_key.as_ref(),
        state.config.tls.enabled,
    )?;
    db::validate_lab_id(&lab_id)?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(&lab_id).await;

    tracing::info!(
        "Starting lab creation: lab_id={}, name={}",
        lab_id,
        manifest.name
    );

    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let Setup {
        qemu_conn,
        db_user,
        node_images,
    } = set_up(
        state,
        &lab_id,
        &lab_dir,
        &ctx.username,
        &manifest.name,
        &progress,
    )
    .await?;

    // ========================================================================
    // PHASE 2: Manifest Validation
    // ========================================================================
    let _ = progress.send_phase(
        data::UpPhase::ManifestValidation,
        "Validating manifest structure".to_string(),
    );
    let build = LabBuild {
        lab_id: &lab_id,
        lab_dir: &lab_dir,
        db: &state.db,
        docker_conn: &state.docker,
        qemu_conn: &qemu_conn,
        config: &state.config,
        progress: &progress,
    };
    let mut plan = plan_lab(&build, &manifest, &db_user, node_images).await?;
    // Held until `up` returns: the containers start during readiness polling.
    let _container_image_guards = mem::take(&mut plan.container_image_guards);
    ensure_no_earlier_attempt(&qemu_conn, &lab_id, &plan.selected_images)?;

    let mut lab_up = LabUp {
        build,
        state,
        operation: &operation,
        manifest: &manifest,
        owner: LabOwner {
            lab_name: &manifest.name,
            user: &db_user,
            details: &details,
        },
        username: &ctx.username,
        plan,
        node_secrets: &node_secrets,
        tailscale_auth_key: tailscale_auth_key.as_ref(),
        tailnet_enabled,
        start_time,
        phases_completed: vec!["Setup".to_string(), "ManifestValidation".to_string()],
        errors: Vec::new(),
    };

    // Phases 1 and 2 create nothing, so their failures return directly. From phase 3 on, a
    // failure rolls the lab back.
    let result = lab_up.create().await;
    state.metrics.operation_duration.record(
        start_time.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "up")],
    );
    if let Err(error) = &result {
        roll_back_failed_up(state, &lab_id, &operation, &progress, error).await;
    }
    result
}

/// The request's manifest, checked before anything else: host access, the description, the
/// files the client read, provisioners, container secret files, and where the lab was started
/// from.
fn check_up_request(
    ctx: &AuthContext,
    manifest: serde_json::Value,
    node_secrets: &BTreeMap<String, Vec<data::SecretFile>>,
    client_host: Option<&str>,
    client_folder: Option<&str>,
) -> Result<topology::Manifest> {
    let manifest: topology::Manifest =
        serde_json::from_value(manifest).context("Failed to deserialize manifest")?;
    require_host_access(ctx, &manifest.nodes)?;
    check_lab_description(manifest.description.as_deref())
        .context("Manifest validation failed: description")?;
    check_client_read_files(&manifest.nodes).context("Manifest validation failed: files")?;
    reject_provisioners(&manifest).context("Manifest validation failed: provisioners")?;
    validate_node_secrets(&manifest, node_secrets)
        .context("Manifest validation failed: container secret files")?;
    validate_lab_origin(client_host, client_folder)
        .context("Request validation failed: lab origin")?;
    Ok(manifest)
}

/// Setup (phase 1): connect to libvirt, load the owner, who must have an SSH key, refuse a lab
/// ID already in use, and load every node image.
#[instrument(skip_all, fields(%lab_id))]
async fn set_up(
    state: &AppState,
    lab_id: &str,
    lab_dir: &str,
    current_user: &str,
    lab_name: &str,
    progress: &ProgressSender,
) -> Result<Setup> {
    let _ = progress.send_phase(data::UpPhase::Setup, "Initializing connections".to_string());

    tracing::info!(lab_id = %lab_id, lab_name = %lab_name, "Connecting to lab infrastructure services");

    tracing::info!(lab_id = %lab_id, "Connected to Docker daemon");

    let qemu_conn = Arc::new(
        state
            .qemu
            .connect()
            .context("Failed to connect to libvirt")?,
    );
    tracing::info!(lab_id = %lab_id, "Connected to libvirt/QEMU");
    tracing::info!(lab_id = %lab_id, "Using the embedded database");

    tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, user = %current_user, "Lab environment prepared");

    let db_user = db::get_user(&state.db, current_user)
        .await
        .context("Failed to get database user")?;
    guest_account::require_ssh_key(&db_user)?;

    // Check if lab already exists (CRITICAL ERROR - fail fast)
    if let Some(lab) = db::find_lab(&state.db, lab_id)
        .await
        .context("Failed to check for an existing lab")?
    {
        return Err(lab_id_taken(
            lab_id,
            &lab.name,
            &lab.user,
            lab_name,
            db_user.id.as_ref(),
        ));
    }

    let _ = progress.send_status("Loading configuration".to_string(), StatusKind::Info);

    // Bulk fetch all node configs from database
    let node_images = db::list_node_images(&state.db)
        .await
        .context("Failed to list node configs from database")?;

    Ok(Setup {
        qemu_conn,
        db_user,
        node_images,
    })
}

/// Refuse to start while disks or VMs of an earlier attempt with this lab ID remain: a startup
/// rollback must never remove resources it did not create. Checked after validation, before any
/// resource is created.
fn ensure_no_earlier_attempt(
    qemu_conn: &QemuConnection,
    lab_id: &str,
    selected_images: &[data::NodeConfig],
) -> Result<()> {
    if selected_images
        .iter()
        .all(|image| image.kind == data::NodeKind::Container)
    {
        return Ok(());
    }
    anyhow::ensure!(
        libvirt::list_lab_volumes(qemu_conn, lab_id)?.is_empty(),
        "Existing disks for this lab ID must be cleaned before startup"
    );
    for domain in qemu_conn.list_all_domains(0)? {
        anyhow::ensure!(
            !util::is_lab_resource(&domain.get_name()?, lab_id),
            "Existing VM for this lab ID must be cleaned before startup"
        );
    }
    Ok(())
}

impl LabUp<'_> {
    /// Start a resource-creating phase: stop if the operation has been stopped, and report it.
    fn start_phase(&self, phase: data::UpPhase, message: String) -> Result<()> {
        self.operation.ensure_active()?;
        let _ = self.build.progress.send_phase(phase, message);
        Ok(())
    }

    /// Record a completed phase for the response.
    fn phase_done(&mut self, name: &str) {
        self.phases_completed.push(name.to_string());
    }

    /// Create the lab's resources and start its nodes (phases 3 to 13).
    async fn create(&mut self) -> Result<data::UpResponse> {
        // ========================================================================
        // PHASE 3: Database Records & Data Structure Building
        // ========================================================================
        self.start_phase(
            data::UpPhase::DatabaseRecords,
            "Creating database records".to_string(),
        )?;
        let mut records = create_node_records(
            &self.build,
            &self.owner,
            &self.plan.nodes_expanded,
            &self.plan.node_images,
            &self.plan.links_detailed,
            &self.plan.bridges_detailed,
        )
        .await?;
        self.phase_done("DatabaseRecords");

        let network = self.set_up_lab_network(&records).await?;
        let links = self.link_nodes(&records).await?;
        let p2p_links: Vec<&data::LabLinkData> = links
            .lab_link_data
            .iter()
            .filter(|l| l.kind == data::BridgeKind::P2p)
            .collect();
        let mut built = self
            .build_nodes(&mut records, &network, &links, &p2p_links)
            .await?;
        let started = self
            .start_nodes(&records, &network, &links, &p2p_links, &mut built)
            .await?;
        Ok(self.response(&records, &links, network, &built, started))
    }

    /// Write the lab files and create the management network, and join the lab to its tailnet
    /// when it has one (phase 4).
    async fn set_up_lab_network(&mut self, records: &NodeRecords) -> Result<LabNetwork> {
        self.start_phase(
            data::UpPhase::LabNetworkSetup,
            "Allocating lab network and creating management network".to_string(),
        )?;
        let lab_id = self.build.lab_id;
        let LabSubnets {
            loopback_subnet,
            management_subnet,
            gateway_ipv4,
            router_ipv4,
            ipv6_management_subnet,
            gateway_ipv6,
            router_ipv6,
            ..
        } = records.subnets;
        let lab_net = management_subnet;

        tracing::info!(
            lab_id = %lab_id,
            subnet = %lab_net,
            gateway = %gateway_ipv4,
            boot_server = %router_ipv4,
            "Allocated lab network subnet"
        );

        let lab_info = data::LabInfo {
            ssh_usernames: self.plan.ssh_usernames.clone(),
            id: lab_id.to_owned(),
            user: self.username.to_owned(),
            name: self.manifest.name.clone(),
            ipv4_network: lab_net,
            ipv4_gateway: gateway_ipv4,
            ipv4_router: router_ipv4,
            loopback_network: loopback_subnet,
            ipv6_network: Some(ipv6_management_subnet),
            ipv6_gateway: Some(gateway_ipv6),
            ipv6_router: Some(router_ipv6),
            description: self.owner.details.description.clone(),
            client_host: self.owner.details.client_host.clone(),
            client_folder: self.owner.details.client_folder.clone(),
        };

        write_lab_files(self.build.lab_dir, &lab_info, self.manifest)?;
        let ssh_key = create_lab_ssh_key(self.build.lab_dir)?;
        let sherpa_user =
            util::sherpa_user(&ssh_key.public).context("Failed to get sherpa user")?;
        let _ = self
            .build
            .progress
            .send_status("Lab SSH key created".to_string(), StatusKind::Done);
        let mgmt_net = management_network(&records.subnets)?;
        let dns = util::default_dns_dual_stack(&lab_net, &ipv6_management_subnet)?;
        create_management_networks(&self.build, &records.subnets).await?;

        let tailnet_status = if self.tailnet_enabled {
            let _ = self.build.progress.send_status(
                "Connecting lab to Tailscale".to_string(),
                StatusKind::Progress,
            );
            let key = self
                .tailscale_auth_key
                .context("Missing Tailscale enrollment credential")?;
            let status =
                tailscale::provision(self.state, &records.lab_record, &lab_info, key).await?;
            for warning in &status.warnings {
                let _ = self
                    .build
                    .progress
                    .send_status(format!("Warning: {warning}"), StatusKind::Info);
                self.errors.push(data::UpError {
                    phase: "Tailscale".into(),
                    message: warning.clone(),
                    is_critical: false,
                });
            }
            Some(status)
        } else {
            None
        };

        self.phase_done("LabNetworkSetup");
        Ok(LabNetwork {
            lab_info,
            ssh_key,
            sherpa_user,
            mgmt_net,
            dns,
            tailnet_status,
        })
    }

    /// Record the point-to-point links (phase 5), create Docker networks for the disabled
    /// interfaces of containers with no link (phase 6), and create the shared bridges (phase 7).
    async fn link_nodes(&mut self, records: &NodeRecords) -> Result<LinkRecords> {
        self.start_phase(
            data::UpPhase::LinkCreation,
            format!(
                "Creating {} point-to-point links",
                self.plan.links_detailed.len()
            ),
        )?;
        let mut links = create_links(
            &self.build,
            &records.lab_record_id,
            &self.plan.links_detailed,
            &records.lab_node_data,
            &records.node_setup_data,
        )
        .await?;
        self.phase_done("LinkCreation");

        self.start_phase(
            data::UpPhase::ContainerNetworks,
            "Creating Docker networks for disabled container interfaces".to_string(),
        )?;
        create_isolated_container_networks(
            &self.build,
            &records.node_setup_data,
            &records.lab_node_data,
            &links.p2p_container_names,
        )
        .await?;
        self.phase_done("ContainerNetworks");

        self.start_phase(
            data::UpPhase::SharedBridges,
            format!(
                "Creating {} shared bridges",
                self.plan.bridges_detailed.len()
            ),
        )?;
        create_shared_bridges(
            &self.build,
            &records.lab_record_id,
            &self.plan.bridges_detailed,
            &records.lab_node_data,
            &links.p2p_container_names,
            &mut links.p2p_container_veths,
        )
        .await?;
        self.phase_done("SharedBridges");
        Ok(links)
    }

    /// Configure every node (phase 8), start the boot container (phase 9), clone the disks
    /// (phase 10), create the VM and unikernel domains (phase 11), attach the eBPF redirect on
    /// VM-to-VM links (11b), and set the isolated bridges down (11c).
    async fn build_nodes(
        &mut self,
        records: &mut NodeRecords,
        network: &LabNetwork,
        links: &LinkRecords,
        p2p_links: &[&data::LabLinkData],
    ) -> Result<BuiltNodes> {
        self.start_phase(
            data::UpPhase::ZtpGeneration,
            "Generating ZTP configurations".to_string(),
        )?;
        let NodeConfigs {
            lab_ca,
            ztp_dir,
            tftp_dir,
            ztp_records,
            container_volumes,
            clone_disks,
            domains,
            unikernel_domains,
            lab_clusters,
        } = configure_nodes(
            &self.build,
            &ConfigInputs {
                node_images: &self.plan.node_images,
                node_setup_data: &records.node_setup_data,
                lab_node_data: &records.lab_node_data,
                mgmt_net: &network.mgmt_net,
                dns: &network.dns,
                subnets: &records.subnets,
                sherpa_user: &network.sherpa_user,
                personal_account: self.plan.personal_account.as_ref(),
                talos_clusters: self.manifest.talos_clusters.as_deref().unwrap_or_default(),
            },
            &mut records.container_nodes,
            &mut records.vm_nodes,
            &mut records.unikernel_nodes,
        )
        .await?;
        self.phase_done("ZtpGeneration");

        self.start_phase(
            data::UpPhase::BootContainers,
            "Creating boot containers and ZTP files".to_string(),
        )?;
        start_boot_container(
            &self.build,
            &ztp_dir,
            &tftp_dir,
            &network.mgmt_net,
            &ztp_records,
        )
        .await?;
        self.phase_done("Sherpa Router");

        self.start_phase(data::UpPhase::DiskCloning, "Cloning disks".to_string())?;
        node_ops::clone_node_disks(
            self.build.qemu_conn.clone(),
            clone_disks,
            self.build.lab_id,
            self.build.progress,
            self.plan.image_guards.clone(),
        )
        .await?;
        self.phase_done("DiskCloning");

        self.start_phase(data::UpPhase::VmCreation, "Creating VMs".to_string())?;
        create_vms(&self.build, domains).await?;
        self.phase_done("VmCreation");
        create_unikernels(&self.build, unikernel_domains).await?;
        // Every disk and domain exists; stop holding the disk images.
        self.plan.image_guards.clear();

        // Now that libvirt has created the tap devices, attach eBPF redirect programs to wire
        // up P2p links between VMs. A link with a container end is attached once that
        // container has started and its veth exists (see readiness polling).
        let p2p = attach_vm_links(
            &self.build,
            p2p_links,
            &links.p2p_container_veths,
            &self.plan.links_detailed,
        )
        .await?;
        self.phase_done("P2pEbpfAttach");

        // Set isolated network bridges DOWN to remove carrier from disabled
        // interfaces, so they show as "not connected" on the node. Containers
        // attach to their bridge later, without running anything in the image.
        set_isolated_bridges_down(&records.node_setup_data).await?;

        Ok(BuiltNodes {
            lab_ca,
            ztp_records,
            container_volumes,
            lab_clusters,
            p2p,
        })
    }

    /// Write the SSH config (phase 12), start the containers and wait for every node (phase
    /// 13), save the SSH files again with the nodes' host keys, and start the lab's services.
    async fn start_nodes(
        &mut self,
        records: &NodeRecords,
        network: &LabNetwork,
        links: &LinkRecords,
        p2p_links: &[&data::LabLinkData],
        built: &mut BuiltNodes,
    ) -> Result<StartedNodes> {
        self.start_phase(
            data::UpPhase::SshConfig,
            "Generating SSH config".to_string(),
        )?;
        let ssh_private_key = write_ssh_files(
            &self.build,
            &built.ztp_records,
            &network.lab_info,
            &network.ssh_key,
        )?;
        let container_link_networks = container_data_attachments(
            self.build.lab_id,
            &records.node_setup_data,
            &records.lab_node_data,
            &links.p2p_container_names,
        )?;
        self.phase_done("SshConfig");

        let ready_timeout_secs = self.manifest.ready_timeout.unwrap_or(READINESS_TIMEOUT);
        self.start_phase(
            data::UpPhase::NodeReadiness,
            format!(
                "Waiting for {} nodes to become ready (up to {} seconds)",
                records.container_nodes.len() + records.vm_nodes.len(),
                ready_timeout_secs
            ),
        )?;
        let talos_clusters = self.manifest.talos_clusters.as_deref().unwrap_or_default();
        let NodesReady {
            nodes,
            errors: readiness_errors,
        } = wait_for_nodes(
            &self.build,
            self.operation,
            &ReadinessInputs {
                container_nodes: &records.container_nodes,
                vm_nodes: &records.vm_nodes,
                unikernel_nodes: &records.unikernel_nodes,
                lab_node_data: &records.lab_node_data,
                ztp_records: &built.ztp_records,
                container_volumes: &built.container_volumes,
                container_link_networks: &container_link_networks,
                p2p_container_veths: &links.p2p_container_veths,
                p2p_links,
                links_detailed: &self.plan.links_detailed,
                node_secrets: self.node_secrets,
                talos_clusters,
                timeout: Duration::from_secs(ready_timeout_secs),
            },
            &mut built.p2p,
        )
        .await?;
        self.errors.extend(readiness_errors);

        // Save the SSH files again with the host keys recorded as nodes became ready.
        let ssh_files = ssh_config::refresh_lab_ssh_files(
            self.build.db,
            self.build.lab_dir,
            &records.lab_record_id,
            &network.lab_info,
            self.build.config.server_ipv4,
        )
        .await?;

        let LabServices {
            clusters,
            keycloaks,
            vaults,
            errors: service_errors,
        } = start_lab_services(
            &self.build,
            self.operation,
            &ServiceInputs {
                manifest_nodes: &self.manifest.nodes,
                container_nodes: &records.container_nodes,
                lab_clusters: &built.lab_clusters,
                lab_ca: &built.lab_ca,
                owner: self.username,
                timeout: Duration::from_secs(ready_timeout_secs),
            },
        )
        .await?;
        self.errors.extend(service_errors);
        self.phase_done("NodeReadiness");

        Ok(StartedNodes {
            nodes,
            ssh_files,
            ssh_private_key,
            clusters,
            keycloaks,
            vaults,
        })
    }

    /// The response: what was created, the nodes, the errors, and what the client saves.
    fn response(
        &self,
        records: &NodeRecords,
        links: &LinkRecords,
        network: LabNetwork,
        built: &BuiltNodes,
        started: StartedNodes,
    ) -> data::UpResponse {
        let summary = data::UpSummary {
            containers_created: records.container_nodes.len(),
            vms_created: records.vm_nodes.len(),
            unikernels_created: records.unikernel_nodes.len(),
            networks_created: 1 + self.plan.bridges_detailed.len(), // management + shared bridges
            bridges_created: self.plan.links_detailed.len() * 2,    // 2 bridges per p2p link
            interfaces_created: links.lab_link_data.len() * 2,      // 2 veth interfaces per link
        };

        let critical_errors = self.errors.iter().filter(|e| e.is_critical).count();
        let warnings = self.errors.iter().filter(|e| !e.is_critical).count();
        let success = critical_errors == 0;
        let total_time = self.start_time.elapsed().as_secs();

        tracing::info!(
            lab_id = %self.build.lab_id,
            lab_name = %self.manifest.name,
            success = success,
            total_time_secs = total_time,
            containers_created = summary.containers_created,
            vms_created = summary.vms_created,
            unikernels_created = summary.unikernels_created,
            networks_created = summary.networks_created,
            bridges_created = summary.bridges_created,
            interfaces_created = summary.interfaces_created,
            critical_errors = critical_errors,
            warnings = warnings,
            phases_completed = self.phases_completed.len(),
            "Lab creation completed"
        );

        data::UpResponse {
            tailscale: network.tailnet_status,
            success,
            lab_info: network.lab_info,
            total_time_secs: total_time,
            phases_completed: self.phases_completed.clone(),
            summary,
            nodes: started.nodes,
            errors: self.errors.clone(),
            ssh_config: started.ssh_files.ssh_config,
            known_hosts: started.ssh_files.known_hosts,
            ssh_private_key: started.ssh_private_key,
            clusters: started.clusters,
            lab_ca: built.lab_ca.pair.cert.clone(),
            vaults: started.vaults,
            keycloaks: started.keycloaks,
        }
    }
}

/// After `up` failed or was stopped, remove what it created, unless a destroy that stopped it
/// is removing the lab already.
#[instrument(skip_all, fields(%lab_id))]
async fn roll_back_failed_up(
    state: &AppState,
    lab_id: &str,
    operation: &LabOperation,
    progress: &ProgressSender,
    error: &anyhow::Error,
) {
    match operation.stop_reason() {
        Some(StopReason::Destroy) => {
            // The lab is being destroyed, which removes everything this run created.
            tracing::info!(lab_id = %lab_id, "Lab creation stopped because the lab is being destroyed");
            return;
        }
        Some(StopReason::Request) => {
            // Stopped by the user: roll back what this run created.
            tracing::info!(lab_id = %lab_id, "Lab creation stopped on request, starting auto-cleanup");
            let _ = progress.send_status(
                "Lab creation stopped, removing what it created...".to_string(),
                StatusKind::Info,
            );
        }
        None => {
            state
                .metrics
                .error_count
                .add(1, &[KeyValue::new("operation.type", "up")]);
            tracing::error!(
                lab_id = %lab_id,
                error = ?error,
                "Lab creation failed after resource creation began, starting auto-cleanup"
            );
            let _ = progress.send_status(
                "Lab creation failed, cleaning up resources...".to_string(),
                StatusKind::Info,
            );
        }
    }

    match clean::clean_lab_resources(lab_id, state).await {
        Ok(clean_response) => {
            if clean_response.success {
                tracing::info!(lab_id = %lab_id, "Auto-cleanup completed successfully");
                let _ = progress.send_status(
                    "Cleanup completed successfully".to_string(),
                    StatusKind::Done,
                );
            } else {
                tracing::warn!(
                    lab_id = %lab_id,
                    errors = ?clean_response.errors,
                    "Auto-cleanup completed with errors"
                );
                let _ = progress.send_status(
                    "Cleanup completed with some errors".to_string(),
                    StatusKind::Info,
                );
            }
        }
        Err(clean_err) => {
            tracing::error!(
                lab_id = %lab_id,
                error = ?clean_err,
                "Auto-cleanup failed"
            );
            let _ = progress.send_status(format!("Cleanup failed: {clean_err}"), StatusKind::Info);
        }
    }
}

/// The error for an `up` whose lab ID is in use. Either this lab is still deployed, or another
/// lab has the same ID: IDs are a hash of the owner and the lab name, so two owners' labs can
/// collide. The other lab belongs to someone else, so it is not named.
fn lab_id_taken(
    lab_id: &str,
    existing_name: &str,
    existing_owner: &RecordId,
    lab_name: &str,
    owner: Option<&RecordId>,
) -> anyhow::Error {
    if existing_name == lab_name && owner == Some(existing_owner) {
        anyhow!(
            "Lab already exists. Please use a different lab ID or destroy the existing lab first.\n Lab name: {lab_name}\n Lab id: {lab_id}"
        )
    } else {
        anyhow!(
            "Lab '{lab_name}' gets the lab ID {lab_id}, which another lab already uses. \
             Lab IDs come from the owner and the lab name, and these collide; rename the lab."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lab_id_another_lab_uses_is_reported_without_naming_it() {
        let alice = RecordId::new("user", "alice".to_string());
        let bob = RecordId::new("user", "bob".to_string());

        let redeployed = lab_id_taken("ab12cd34", "core", &alice, "core", Some(&alice));
        assert!(
            redeployed.to_string().contains("destroy the existing lab"),
            "{redeployed}"
        );

        let collision = lab_id_taken("ab12cd34", "x", &bob, "core", Some(&alice)).to_string();
        assert!(collision.contains("rename the lab"), "{collision}");
        assert!(!collision.contains("'x'"), "{collision}");
    }
}
