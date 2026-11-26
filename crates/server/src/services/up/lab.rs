//! The `up_lab` orchestrator: setup (phase 1), each phase in turn, and rolling back a lab
//! whose `up` failed.

use std::collections::{BTreeMap, HashMap};
use std::mem;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use opentelemetry::KeyValue;
use tracing::instrument;

use super::context::LabBuild;
use super::links::{LinkRecords, P2pAttachment, attach_vm_links, create_links};
use super::networks::{
    ReservedLab, check_existing_docker_management_network, container_data_attachments,
    create_isolated_container_networks, create_management_networks, create_shared_bridges,
    lab_id_taken, management_network, reserve_lab, set_isolated_bridges_down,
};
use super::plan::{CheckedNodes, LabPlan, check_node_images, plan_lab};
use super::readiness::{NodesReady, ReadinessInputs, wait_for_nodes};
use super::records::{NodeRecords, create_node_records, store_ssh_usernames};
use super::runtime::{create_node_vms, create_unikernels, create_vms};
use super::services::{LabServices, ServiceInputs, record_ready_workers, start_lab_services};
use super::ztp::{ConfigInputs, NodeConfigs, configure_nodes, start_boot_container};
use crate::auth::context::AuthContext;
use crate::auth::policy::{require_host_access, require_lab_starter};
use crate::daemon::state::AppState;
use crate::services::clean;
use crate::services::console;
use crate::services::error::{ServiceError, caller_message};
use crate::services::guest_account::{self, GuestAccount};
use crate::services::lab_definition;
use crate::services::lab_lifecycle;
use crate::services::lab_operations::{self, LabHold, LabOperation, Operation, StopReason};
use crate::services::node_ops;
use crate::services::node_vm;
use crate::services::progress::ProgressSender;
use crate::services::ssh_config::{self, LabSshFiles};
use crate::services::surreal_db;
use crate::services::tailscale;
use crate::services::talos::LabCluster;
use crate::tls::lab_ca::LabCa;
use crate::validate::{reject_provisioners, validate_lab_origin, validate_node_secrets};

use libvirt::QemuConnection;
use shared::data;
use shared::data::{LabEvent, LabSubnets, StatusKind, TailnetAuthKey, TailnetStatus};
use shared::konst::{READINESS_TIMEOUT, SHERPA_LABS_PATH};
use shared::util;
use topology::validate::{
    check_client_read_files, check_lab_description, check_lab_name, check_ready_timeout,
};

/// What setup (phase 1) gives the later phases: the libvirt connection, the owner's record,
/// every node image, the manifest's nodes with their images checked, and the lab's reservation.
struct Setup {
    qemu_conn: Arc<QemuConnection>,
    owner_account: GuestAccount,
    checked: CheckedNodes,
    reserved: ReservedLab,
}

/// One `up`: the lab being built, what phases 1 and 2 decided, and the phases completed and
/// non-critical errors so far. The resource-creating phases (3 to 13) are its methods.
struct LabUp<'a> {
    build: LabBuild<'a>,
    state: &'a AppState,
    operation: &'a LabOperation,
    manifest: &'a topology::Manifest,
    definition: &'a topology::Manifest,
    details: &'a db::LabDetails,
    username: &'a str,
    plan: LabPlan,
    node_secrets: &'a BTreeMap<String, Vec<data::SecretFile>>,
    tailscale_auth_key: Option<&'a TailnetAuthKey>,
    tailnet_enabled: bool,
    start_time: Instant,
    phases_completed: Vec<String>,
    errors: Vec<data::UpError>,
}

/// The lab's management network and what was made on it (phase 4).
struct LabNetwork {
    lab_info: data::LabInfo,
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
    clusters: Vec<data::ClusterInfo>,
    keycloaks: Vec<data::KeycloakInfo>,
    surreal_dbs: Vec<data::SurrealDbInfo>,
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
        as_written,
        file_contents,
    } = request;
    require_lab_starter(ctx)?;
    let CheckedManifest {
        manifest,
        definition,
    } = check_up_request(
        ctx,
        &manifest,
        file_contents.as_ref(),
        as_written.as_ref(),
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
    db::validate_lab_id(&lab_id).map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(&lab_id, Operation::Up).await;

    tracing::info!(
        "Starting lab creation: lab_id={}, name={}",
        lab_id,
        manifest.name
    );

    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let Setup {
        qemu_conn,
        owner_account: owner,
        checked,
        reserved,
    } = set_up(
        state,
        &lab_id,
        &lab_dir,
        &ctx.username,
        &manifest,
        &details,
        &progress,
    )
    .await?;

    // ========================================================================
    // PHASE 2: Manifest Validation
    // ========================================================================
    let build = LabBuild {
        lab_id: &lab_id,
        lab_dir: &lab_dir,
        db: &state.db,
        docker_conn: &state.docker,
        qemu_conn: &qemu_conn,
        qemu: &state.qemu,
        config: &state.config,
        progress: &progress,
    };
    let mut plan = validate(
        &build, ctx, &operation, &manifest, owner, checked, &reserved,
    )
    .await?;
    // Held until `up` returns: the containers start during readiness polling.
    let _container_image_guards = mem::take(&mut plan.container_image_guards);

    let mut lab_up = LabUp {
        build,
        state,
        operation: &operation,
        manifest: &manifest,
        definition: &definition,
        details: &details,
        username: &ctx.username,
        plan,
        node_secrets: &node_secrets,
        tailscale_auth_key: tailscale_auth_key.as_ref(),
        tailnet_enabled,
        start_time,
        phases_completed: vec!["Setup".to_string(), "ManifestValidation".to_string()],
        errors: Vec::new(),
    };

    // From phase 3 on, a failure rolls the lab back.
    let result = lab_up.create(reserved).await;
    state.metrics.operation_duration.record(
        start_time.elapsed().as_secs_f64(),
        &[KeyValue::new("operation.type", "up")],
    );
    if let Err(error) = &result {
        Box::pin(roll_back_failed_up(state, &operation, &progress, error)).await;
    }
    result
}

/// Validation (phase 2): plan the lab, and refuse to start over an earlier attempt's disks or
/// VMs, or over a Docker management network of the lab's name that is not the lab's. Nothing has
/// been created yet, so a failure gives the reservation back.
async fn validate(
    build: &LabBuild<'_>,
    requester: &AuthContext,
    operation: &LabOperation,
    manifest: &topology::Manifest,
    owner_account: GuestAccount,
    checked: CheckedNodes,
    reserved: &ReservedLab,
) -> Result<LabPlan> {
    let _ = build.progress.send_phase(
        data::UpPhase::ManifestValidation,
        "Validating manifest structure".to_string(),
    );
    let planned = plan_and_check(
        build,
        requester,
        manifest,
        owner_account,
        checked,
        &reserved.subnets,
    )
    .await;
    if planned.is_err()
        && let Err(error) = lab_lifecycle::record(build.db, operation, LabEvent::Released).await
    {
        tracing::error!(lab_id = %build.lab_id, error = ?error, "Failed to release the lab reservation");
    }
    planned
}

/// The manifest `up` builds the lab from, as the client sent it, and the lab's definition.
struct CheckedManifest {
    manifest: topology::Manifest,
    definition: topology::Manifest,
}

/// The request's manifest, checked before anything else: the lab name, host access, the ready
/// timeout, the description, the files the client read, provisioners, container secret files,
/// and where the lab was started from; and the lab's definition, built from it and the parts as
/// written.
fn check_up_request(
    ctx: &AuthContext,
    manifest: &serde_json::Value,
    file_contents: Option<&serde_json::Value>,
    as_written: Option<&serde_json::Value>,
    node_secrets: &BTreeMap<String, Vec<data::SecretFile>>,
    client_host: Option<&str>,
    client_folder: Option<&str>,
) -> Result<CheckedManifest> {
    let manifest = lab_definition::parse_manifest(manifest, file_contents)?;
    check_lab_name(&manifest.name)
        .context("Manifest validation failed: lab name")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    require_host_access(ctx, &manifest.nodes)?;
    check_ready_timeout(manifest.ready_timeout)
        .context("Manifest validation failed: ready timeout")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    check_lab_description(manifest.description.as_deref())
        .context("Manifest validation failed: description")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    check_client_read_files(&manifest.nodes)
        .context("Manifest validation failed: files")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    reject_provisioners(&manifest)
        .context("Manifest validation failed: provisioners")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    let as_written = lab_definition::parse_as_written(as_written)?;
    let definition = lab_definition::definition(&manifest, as_written.as_ref())
        .context("Manifest validation failed: the manifest as written")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    validate_node_secrets(&manifest, node_secrets)
        .context("Manifest validation failed: container secret files")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    validate_lab_origin(client_host, client_folder)
        .context("Request validation failed: lab origin")
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    Ok(CheckedManifest {
        manifest,
        definition,
    })
}

/// Setup (phase 1): load the owner, refuse a lab ID already in use, resolve the owner's login on
/// lab nodes, which needs an SSH key, a device password and a name nodes can take, load every
/// node image and check the manifest's nodes against them, connect to libvirt, then reserve the
/// lab.
///
/// The lab ID is checked before libvirt or the host is touched, so a second `up` of a lab fails
/// fast with the same answer on any host. The reservation checks it again: two `up`s of one lab
/// can both pass the first check, and only one record is inserted. The nodes' images are also
/// checked before libvirt or Docker is touched, so a missing image gets the same answer whether
/// or not the hypervisor is reachable.
#[instrument(skip_all, fields(%lab_id))]
async fn set_up(
    state: &AppState,
    lab_id: &str,
    lab_dir: &str,
    current_user: &str,
    manifest: &topology::Manifest,
    details: &db::LabDetails,
    progress: &ProgressSender,
) -> Result<Setup> {
    let lab_name = manifest.name.as_str();
    let _ = progress.send_phase(data::UpPhase::Setup, "Initializing connections".to_string());

    tracing::info!(lab_id = %lab_id, lab_name = %lab_name, "Connecting to lab infrastructure services");

    let db_user = db::get_user(&state.db, current_user)
        .await
        .context("Failed to get database user")?;
    if let Some(existing) = db::find_lab(&state.db, lab_id)
        .await
        .context("Failed to check for an existing lab")?
    {
        return Err(lab_id_taken(
            lab_id,
            &existing,
            lab_name,
            db_user.id.as_ref(),
        ));
    }
    guest_account::require_ssh_key(&db_user)?;
    let device_password = guest_account::require_device_password(&state.db, &db_user).await?;
    let owner_account = guest_account::from_user(&db_user, device_password)?;

    let _ = progress.send_status("Loading configuration".to_string(), StatusKind::Info);

    // Bulk fetch all node configs from database
    let node_images = db::list_node_images(&state.db)
        .await
        .context("Failed to list node configs from database")?;
    // Before libvirt or Docker is touched.
    let checked = check_node_images(manifest, node_images, &state.config.images_dir)?;

    let qemu_conn = Arc::new(
        state
            .qemu
            .connect()
            .context("Failed to connect to libvirt")?,
    );
    tracing::info!(lab_id = %lab_id, "Connected to libvirt/QEMU");

    tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, user = %current_user, "Lab environment prepared");

    // Last, so nothing after it in this phase can fail: the record is the reservation, and a
    // second `up` of this lab fails here.
    let reserved = reserve_lab(
        &state.db,
        &state.config,
        lab_name,
        lab_id,
        &db_user,
        details,
    )
    .await?;

    Ok(Setup {
        qemu_conn,
        owner_account,
        checked,
        reserved,
    })
}

/// Plan the lab and run the checks that must pass before anything is created.
async fn plan_and_check(
    build: &LabBuild<'_>,
    requester: &AuthContext,
    manifest: &topology::Manifest,
    owner_account: GuestAccount,
    checked: CheckedNodes,
    subnets: &LabSubnets,
) -> Result<LabPlan> {
    let plan = plan_lab(build, requester, manifest, owner_account, checked).await?;
    let model_kinds = plan
        .selected_images
        .iter()
        .map(data::NodeConfig::model_kind)
        .collect::<Result<Vec<_>>>()?;
    node_vm::require_base_image(&build.config.images_dir, &model_kinds)?;
    ensure_no_earlier_attempt(build.qemu_conn, build.lab_id, &plan.selected_images)?;
    check_existing_docker_management_network(build.docker_conn, build.lab_id, subnets).await?;
    Ok(plan)
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
        .all(|image| image.runtime_kind() == data::NodeKind::Container)
    {
        return Ok(());
    }
    if !libvirt::list_lab_volumes(qemu_conn, lab_id)?.is_empty() {
        return Err(ServiceError::Conflict(
            "Existing disks for this lab ID must be cleaned before startup".to_owned(),
        )
        .into());
    }
    for domain in qemu_conn.list_all_domains(0)? {
        if util::is_lab_resource(&domain.get_name()?, lab_id) {
            return Err(ServiceError::Conflict(
                "Existing VM for this lab ID must be cleaned before startup".to_owned(),
            )
            .into());
        }
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

    /// Create the lab's resources and start its nodes (phases 3 to 13), then record the lab
    /// built.
    async fn create(&mut self, reserved: ReservedLab) -> Result<data::UpResponse> {
        // ========================================================================
        // PHASE 3: Database Records & Data Structure Building
        // ========================================================================
        self.start_phase(
            data::UpPhase::DatabaseRecords,
            "Creating database records".to_string(),
        )?;
        let mut records = create_node_records(
            &self.build,
            self.definition,
            reserved,
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
        lab_lifecycle::record(self.build.db, self.operation, LabEvent::Built)
            .await
            .context("Failed to record the lab built")?;
        Ok(self.response(&records, &links, network, &built, started))
    }

    /// Create the lab's folder and management network, record each node's SSH account, and join the lab to its tailnet
    /// when it has one (phase 4).
    async fn set_up_lab_network(&mut self, records: &NodeRecords) -> Result<LabNetwork> {
        self.start_phase(
            data::UpPhase::LabNetworkSetup,
            "Allocating lab network and creating management network".to_string(),
        )?;
        let lab_id = self.build.lab_id;
        let LabSubnets {
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
            ipv6_network: Some(ipv6_management_subnet),
            ipv6_gateway: Some(gateway_ipv6),
            ipv6_router: Some(router_ipv6),
            description: self.details.description.clone(),
            client_host: self.details.client_host.clone(),
            client_folder: self.details.client_folder.clone(),
        };

        util::create_dir(self.build.lab_dir)?;
        console::create_console_dir(lab_id)?;
        store_ssh_usernames(
            self.build.db,
            &records.lab_node_data,
            &self.plan.ssh_usernames,
        )
        .await?;
        let mgmt_net = management_network(&records.subnets)?;
        let dns = util::default_dns(&mgmt_net);
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
            mgmt_net,
            dns,
            tailnet_status,
        })
    }

    /// Record the links (phase 5), make isolated container networks (6) and the bridges (7).
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
            &self.plan.bridges_detailed,
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
            &links.veth_container_names,
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
            ztp_records,
            container_volumes,
            clone_disks,
            domains,
            unikernel_domains,
            lab_clusters,
        } = configure_nodes(
            &self.build,
            &ConfigInputs {
                lab_record_id: &records.lab_record_id,
                node_images: &self.plan.node_images,
                node_setup_data: &records.node_setup_data,
                lab_node_data: &records.lab_node_data,
                mgmt_net: &network.mgmt_net,
                dns: &network.dns,
                owner_account: &self.plan.owner_account,
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
        start_boot_container(&self.build, &ztp_dir, &network.mgmt_net, &ztp_records).await?;
        self.phase_done("Sherpa Router");

        self.start_phase(data::UpPhase::DiskCloning, "Cloning disks".to_string())?;
        let boot_dirs: Vec<PathBuf> = records
            .vm_nodes
            .iter()
            .map(|node| PathBuf::from(node_ops::vm_boot_dir(self.build.lab_dir, &node.name)))
            .collect();
        node_ops::clone_node_disks(
            self.build.qemu_conn.clone(),
            clone_disks,
            self.build.lab_id,
            self.build.progress,
            self.plan.image_guards.clone(),
            &boot_dirs,
        )
        .await?;
        self.phase_done("DiskCloning");

        self.start_phase(data::UpPhase::VmCreation, "Creating VMs".to_string())?;
        create_vms(&self.build, domains).await?;
        create_node_vms(
            &self.build,
            &records.container_nodes,
            &records.lab_node_data,
            &links.p2p_container_veths,
        )
        .await?;
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

    /// Prepare the containers' data networks (phase 12), start the containers and wait for
    /// every node (phase 13), build the SSH files with the nodes' host keys from the database,
    /// and start the lab's services.
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
        let container_link_networks = container_data_attachments(
            self.build.lab_id,
            &records.node_setup_data,
            &records.lab_node_data,
            &links.veth_container_names,
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
            mut nodes,
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
                dns: &network.dns,
                timeout: Duration::from_secs(ready_timeout_secs),
            },
            &mut built.p2p,
        )
        .await?;
        self.errors.extend(readiness_errors);

        // The SSH files for the response, with the host keys recorded as nodes became ready.
        let ssh_files =
            ssh_config::lab_ssh_files(self.build.db, &records.lab_record_id, &network.lab_info)
                .await?;

        let LabServices {
            clusters,
            ready_workers,
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
                timeout: Duration::from_secs(ready_timeout_secs),
            },
        )
        .await?;
        self.errors.extend(service_errors);
        record_ready_workers(
            self.build.db,
            self.operation,
            &records.lab_node_data,
            &ready_workers,
            &mut nodes,
        )
        .await?;
        // The root users the lab's SurrealDB nodes were created with.
        let surreal_dbs = surreal_db::read_surreal_db_credentials(&surreal_db::surreal_db_dir(
            self.build.lab_id,
        ))?;
        self.phase_done("NodeReadiness");

        Ok(StartedNodes {
            nodes,
            ssh_files,
            clusters,
            keycloaks,
            surreal_dbs,
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
            clusters: started.clusters,
            lab_ca: built.lab_ca.pair.cert.clone(),
            vaults: started.vaults,
            keycloaks: started.keycloaks,
            surreal_dbs: started.surreal_dbs,
        }
    }
}

/// After `up` failed or was stopped, remove what it created, unless a destroy that stopped it
/// is removing the lab already.
/// Remove what a failed `up` created, through the lab's `Destroying` phase. A rollback that
/// leaves something behind leaves the lab `DestroyFailed`, so its owner can destroy it again.
/// A lab being destroyed is left to the destroy.
#[instrument(skip_all, fields(lab_id = %operation.lab_id()))]
async fn roll_back_failed_up(
    state: &AppState,
    operation: &LabOperation,
    progress: &ProgressSender,
    error: &anyhow::Error,
) {
    let lab_id = operation.lab_id();
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

    match clean::clean_lab_resources(state, operation).await {
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
            let _ = progress.send_status(
                format!(
                    "Cleanup failed: {}",
                    caller_message(&clean_err, "Auto-cleanup failed")
                ),
                StatusKind::Info,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::request_parse::InvalidRequest;

    #[test]
    fn api_errors_014_an_up_whose_manifest_does_not_parse_names_the_path_never_the_value() {
        let ctx = AuthContext::new("dev".to_string(), false, "s".to_string());
        let manifest = serde_json::json!({
            "name": "core",
            "nodes": [{ "name": "r1", "model": "hunter2-s3cret" }],
        });
        let Err(error) =
            check_up_request(&ctx, &manifest, None, None, &BTreeMap::new(), None, None)
        else {
            panic!("parsed");
        };
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::RequestUnreadable {
                what: "manifest",
                refusal: InvalidRequest::Invalid {
                    path: Some("nodes[0].model".to_owned())
                },
            })
        );
    }
}
