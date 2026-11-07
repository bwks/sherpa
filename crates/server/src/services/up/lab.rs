//! The `up_lab` orchestrator: setup (phase 1), each phase in turn, and rolling back a lab
//! whose `up` failed.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use opentelemetry::KeyValue;
use tracing::instrument;

use super::context::LabBuild;
use super::links::{LinkRecords, attach_vm_links, create_links};
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
use crate::services::node_ops;
use crate::services::progress::ProgressSender;
use crate::services::ssh_config;
use crate::services::tailscale;
use crate::validate::{reject_provisioners, validate_lab_origin, validate_node_secrets};

use shared::data;
use shared::data::{LabSubnets, RecordId, StatusKind};
use shared::konst::{READINESS_TIMEOUT, SHERPA_LABS_PATH};
use shared::util;
use topology::validate::check_client_read_files;

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
    let mut phases_completed = Vec::new();
    let mut errors = Vec::new();

    let lab_id = &request.lab_id;
    require_lab_starter(ctx)?;

    // Deserialize manifest from JSON Value
    let manifest: topology::Manifest =
        serde_json::from_value(request.manifest).context("Failed to deserialize manifest")?;
    require_host_access(ctx, &manifest.nodes)?;
    check_client_read_files(&manifest.nodes).context("Manifest validation failed: files")?;
    reject_provisioners(&manifest).context("Manifest validation failed: provisioners")?;
    validate_node_secrets(&manifest, &request.node_secrets)
        .context("Manifest validation failed: container secret files")?;
    validate_lab_origin(
        request.client_host.as_deref(),
        request.client_folder.as_deref(),
    )
    .context("Request validation failed: lab origin")?;
    let origin = db::LabOrigin {
        client_host: request.client_host.clone(),
        client_folder: request.client_folder.clone(),
    };

    let tailnet_enabled = tailscale::validate_enrollment(
        &manifest,
        request.tailscale_auth_key.as_ref(),
        state.config.tls.enabled,
    )?;
    db::validate_lab_id(lab_id)?;
    // Destroying the lab stops this operation; see `lab_operations`.
    let operation = lab_operations::begin(lab_id).await;

    tracing::info!(
        "Starting lab creation: lab_id={}, name={}",
        lab_id,
        manifest.name
    );

    // ========================================================================
    // PHASE 1: Setup & Connections
    // ========================================================================
    let _ = progress.send_phase(data::UpPhase::Setup, "Initializing connections".to_string());

    tracing::info!(lab_id = %lab_id, lab_name = %manifest.name, "Connecting to lab infrastructure services");

    let sherpa_user = util::sherpa_user().context("Failed to get sherpa user")?;
    let lab_dir = format!("{SHERPA_LABS_PATH}/{lab_id}");
    let current_user = &ctx.username;

    // Get connections from AppState
    let docker_conn = state.docker.clone();
    tracing::info!(lab_id = %lab_id, "Connected to Docker daemon");

    let qemu_conn = Arc::new(
        state
            .qemu
            .connect()
            .context("Failed to connect to libvirt")?,
    );
    tracing::info!(lab_id = %lab_id, "Connected to libvirt/QEMU");

    // Use the shared database connection from AppState
    let db = state.db.clone();
    tracing::info!(lab_id = %lab_id, "Using the embedded database");

    tracing::debug!(lab_id = %lab_id, lab_dir = %lab_dir, user = %current_user, "Lab environment prepared");

    let db_user = db::get_user(&db, current_user)
        .await
        .context("Failed to get database user")?;
    guest_account::require_ssh_key(&db_user)?;

    // Check if lab already exists (CRITICAL ERROR - fail fast)
    if let Some(lab) = db::find_lab(&db, lab_id)
        .await
        .context("Failed to check for an existing lab")?
    {
        return Err(lab_id_taken(
            lab_id,
            &lab.name,
            &lab.user,
            &manifest.name,
            db_user.id.as_ref(),
        ));
    }

    let _ = progress.send_status("Loading configuration".to_string(), StatusKind::Info);
    let config = state.config.clone();

    // Bulk fetch all node configs from database
    let node_images = db::list_node_images(&db)
        .await
        .context("Failed to list node configs from database")?;

    phases_completed.push("Setup".to_string());

    // ========================================================================
    // PHASE 2: Manifest Validation
    // ========================================================================
    let _ = progress.send_phase(
        data::UpPhase::ManifestValidation,
        "Validating manifest structure".to_string(),
    );
    let build = LabBuild {
        lab_id,
        lab_dir: &lab_dir,
        db: &db,
        docker_conn: &docker_conn,
        qemu_conn: &qemu_conn,
        config: &config,
        progress: &progress,
    };
    let LabPlan {
        node_images,
        nodes_expanded,
        selected_images,
        mut image_guards,
        // Held until `up` returns: the containers start during readiness polling. A `_`
        // pattern here would drop the leases at once.
        container_image_guards: _container_image_guards,
        personal_account,
        ssh_usernames,
        links_detailed,
        bridges_detailed,
    } = plan_lab(&build, &manifest, &db_user, node_images).await?;
    phases_completed.push("ManifestValidation".to_string());

    // Startup rollback must never remove resources from a previous attempt.
    if selected_images
        .iter()
        .any(|image| image.kind != data::NodeKind::Container)
    {
        anyhow::ensure!(
            libvirt::list_lab_volumes(&qemu_conn, lab_id)?.is_empty(),
            "Existing disks for this lab ID must be cleaned before startup"
        );
        for domain in qemu_conn.list_all_domains(0)? {
            anyhow::ensure!(
                !util::is_lab_resource(&domain.get_name()?, lab_id),
                "Existing VM for this lab ID must be cleaned before startup"
            );
        }
    }

    // Wrap resource-creating phases in a block so we can auto-clean on failure.
    // Phases 1-2 (Setup, ManifestValidation) don't create resources, so failures
    // there propagate directly without cleanup.
    let resource_creation = async {
        // ========================================================================
        // PHASE 3: Database Records & Data Structure Building
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::DatabaseRecords,
            "Creating database records".to_string(),
        );
        let NodeRecords {
            lab_record,
            lab_record_id,
            subnets,
            mut container_nodes,
            mut unikernel_nodes,
            mut vm_nodes,
            lab_node_data,
            node_setup_data,
        } = create_node_records(
            &build,
            &LabOwner {
                lab_name: &manifest.name,
                user: &db_user,
                origin: &origin,
            },
            &nodes_expanded,
            &node_images,
            &links_detailed,
            &bridges_detailed,
        )
        .await?;
        let LabSubnets {
            loopback_subnet,
            management_subnet,
            gateway_ipv4,
            router_ipv4,
            ipv6_management_subnet,
            gateway_ipv6,
            router_ipv6,
            ..
        } = subnets;
        phases_completed.push("DatabaseRecords".to_string());

        // ========================================================================
        // PHASE 4: Lab Network Setup
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::LabNetworkSetup,
            "Allocating lab network and creating management network".to_string(),
        );

        let lab_net = management_subnet;

        tracing::info!(
            lab_id = %lab_id,
            subnet = %lab_net,
            gateway = %gateway_ipv4,
            boot_server = %router_ipv4,
            "Allocated lab network subnet"
        );

        let lab_info = data::LabInfo {
            ssh_usernames: ssh_usernames.clone(),
            id: lab_id.to_string(),
            user: current_user.clone(),
            name: manifest.name.clone(),
            ipv4_network: lab_net,
            ipv4_gateway: gateway_ipv4,
            ipv4_router: router_ipv4,
            loopback_network: loopback_subnet,
            ipv6_network: Some(ipv6_management_subnet),
            ipv6_gateway: Some(gateway_ipv6),
            ipv6_router: Some(router_ipv6),
            client_host: origin.client_host.clone(),
            client_folder: origin.client_folder.clone(),
        };

        write_lab_files(&lab_dir, &lab_info, &manifest)?;
        let mgmt_net = management_network(&subnets)?;
        let dns = util::default_dns_dual_stack(&lab_net, &ipv6_management_subnet)?;
        create_management_networks(&build, &subnets).await?;

        let tailnet_status = if tailnet_enabled {
            let _ = progress.send_status(
                "Connecting lab to Tailscale".to_string(),
                StatusKind::Progress,
            );
            let key = request
                .tailscale_auth_key
                .as_ref()
                .context("Missing Tailscale enrollment credential")?;
            let status = tailscale::provision(state, &lab_record, &lab_info, key).await?;
            for warning in &status.warnings {
                let _ = progress.send_status(format!("Warning: {warning}"), StatusKind::Info);
                errors.push(data::UpError {
                    phase: "Tailscale".into(),
                    message: warning.clone(),
                    is_critical: false,
                });
            }
            Some(status)
        } else {
            None
        };

        phases_completed.push("LabNetworkSetup".to_string());

        // ========================================================================
        // PHASE 5: Point-to-Point Link Creation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::LinkCreation,
            format!("Creating {} point-to-point links", links_detailed.len()),
        );
        let LinkRecords {
            lab_link_data,
            mut p2p_container_veths,
            p2p_container_names,
        } = create_links(
            &build,
            &lab_record_id,
            &links_detailed,
            &lab_node_data,
            &node_setup_data,
        )
        .await?;
        phases_completed.push("LinkCreation".to_string());

        // ========================================================================
        // PHASE 6: Docker networks for disabled interfaces of containers with no links
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::ContainerNetworks,
            "Creating Docker networks for disabled container interfaces".to_string(),
        );
        create_isolated_container_networks(
            &build,
            &node_setup_data,
            &lab_node_data,
            &p2p_container_names,
        )
        .await?;
        phases_completed.push("ContainerNetworks".to_string());

        // ========================================================================
        // PHASE 7: Shared Bridge Creation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::SharedBridges,
            format!("Creating {} shared bridges", bridges_detailed.len()),
        );
        create_shared_bridges(
            &build,
            &lab_record_id,
            &bridges_detailed,
            &lab_node_data,
            &p2p_container_names,
            &mut p2p_container_veths,
        )
        .await?;
        phases_completed.push("SharedBridges".to_string());

        // ========================================================================
        // PHASE 8: ZTP Configuration Generation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::ZtpGeneration,
            "Generating ZTP configurations".to_string(),
        );
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
            &build,
            &ConfigInputs {
                node_images: &node_images,
                node_setup_data: &node_setup_data,
                lab_node_data: &lab_node_data,
                mgmt_net: &mgmt_net,
                dns: &dns,
                subnets: &subnets,
                sherpa_user: &sherpa_user,
                personal_account: personal_account.as_ref(),
                talos_clusters: manifest.talos_clusters.as_deref().unwrap_or_default(),
            },
            &mut container_nodes,
            &mut vm_nodes,
            &mut unikernel_nodes,
        )
        .await?;
        phases_completed.push("ZtpGeneration".to_string());

        // ========================================================================
        // PHASE 9: Sherpa Router & ZTP File Creation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::BootContainers,
            "Creating boot containers and ZTP files".to_string(),
        );
        start_boot_container(&build, &ztp_dir, &tftp_dir, &mgmt_net, &ztp_records).await?;
        phases_completed.push("Sherpa Router".to_string());

        // ========================================================================
        // PHASE 10: Disk Cloning (For VMs and Unikernels)
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(data::UpPhase::DiskCloning, "Cloning disks".to_string());

        node_ops::clone_node_disks(
            qemu_conn.clone(),
            clone_disks,
            lab_id,
            &progress,
            image_guards.clone(),
        )
        .await?;

        phases_completed.push("DiskCloning".to_string());

        // ========================================================================
        // PHASE 11: VM Creation
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(data::UpPhase::VmCreation, "Creating VMs".to_string());

        create_vms(&build, domains).await?;
        phases_completed.push("VmCreation".to_string());
        create_unikernels(&build, unikernel_domains).await?;
        // Every disk and domain exists; stop holding the disk images.
        image_guards.clear();

        // ========================================================================
        // PHASE 11b: Attach eBPF redirect on P2p links
        // ========================================================================
        // Now that libvirt has created the tap devices, attach eBPF redirect programs to wire
        // up P2p links between VMs. A link with a container end is attached once that
        // container has started and its veth exists (see readiness polling).
        let p2p_links: Vec<&data::LabLinkData> = lab_link_data
            .iter()
            .filter(|l| l.kind == data::BridgeKind::P2p)
            .collect();
        let mut p2p =
            attach_vm_links(&build, &p2p_links, &p2p_container_veths, &links_detailed).await?;

        phases_completed.push("P2pEbpfAttach".to_string());

        // ========================================================================
        // PHASE 11c: Set isolated bridges DOWN
        // ========================================================================
        // Set isolated network bridges DOWN to remove carrier from disabled
        // interfaces, so they show as "not connected" on the node. Containers
        // attach to their bridge later, without running anything in the image.
        set_isolated_bridges_down(&node_setup_data).await?;

        // ========================================================================
        // PHASE 12: SSH Config & Network Map Building
        // ========================================================================
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::SshConfig,
            "Generating SSH config".to_string(),
        );

        let ssh_private_key = write_ssh_files(&build, &ztp_records, &lab_info)?;
        let container_link_networks = container_data_attachments(
            lab_id,
            &node_setup_data,
            &lab_node_data,
            &p2p_container_names,
        )?;
        phases_completed.push("SshConfig".to_string());

        // ========================================================================
        // PHASE 13: Node Readiness Polling
        // ========================================================================
        let ready_timeout_secs = manifest.ready_timeout.unwrap_or(READINESS_TIMEOUT);
        operation.ensure_active()?;
        let _ = progress.send_phase(
            data::UpPhase::NodeReadiness,
            format!(
                "Waiting for {} nodes to become ready (up to {} seconds)",
                container_nodes.len() + vm_nodes.len(),
                ready_timeout_secs
            ),
        );

        let NodesReady {
            nodes: node_info_list,
            errors: readiness_errors,
        } = wait_for_nodes(
            &build,
            &operation,
            &ReadinessInputs {
                container_nodes: &container_nodes,
                vm_nodes: &vm_nodes,
                unikernel_nodes: &unikernel_nodes,
                lab_node_data: &lab_node_data,
                ztp_records: &ztp_records,
                container_volumes: &container_volumes,
                container_link_networks: &container_link_networks,
                p2p_container_veths: &p2p_container_veths,
                p2p_links: &p2p_links,
                links_detailed: &links_detailed,
                node_secrets: &request.node_secrets,
                talos_clusters: manifest.talos_clusters.as_deref().unwrap_or_default(),
                timeout: Duration::from_secs(ready_timeout_secs),
            },
            &mut p2p,
        )
        .await?;
        errors.extend(readiness_errors);

        // Save the SSH files again with the host keys recorded as nodes became ready.
        let ssh_files = ssh_config::refresh_lab_ssh_files(
            &db,
            &lab_dir,
            &lab_record_id,
            &lab_info,
            config.server_ipv4,
        )
        .await?;

        let LabServices {
            clusters,
            keycloaks,
            vaults,
            errors: service_errors,
        } = start_lab_services(
            &build,
            &operation,
            &ServiceInputs {
                manifest_nodes: &manifest.nodes,
                container_nodes: &container_nodes,
                lab_clusters: &lab_clusters,
                lab_ca: &lab_ca,
                owner: current_user,
                timeout: Duration::from_secs(ready_timeout_secs),
            },
        )
        .await?;
        errors.extend(service_errors);
        phases_completed.push("NodeReadiness".to_string());

        // ========================================================================
        // Build Response
        // ========================================================================

        let summary = data::UpSummary {
            containers_created: container_nodes.len(),
            vms_created: vm_nodes.len(),
            unikernels_created: unikernel_nodes.len(),
            networks_created: 1 + bridges_detailed.len(), // management + shared bridges
            bridges_created: links_detailed.len() * 2,    // 2 bridges per p2p link
            interfaces_created: lab_link_data.len() * 2,  // 2 veth interfaces per link
        };

        let critical_errors = errors.iter().filter(|e| e.is_critical).count();
        let warnings = errors.iter().filter(|e| !e.is_critical).count();
        let success = critical_errors == 0;
        let total_time = start_time.elapsed().as_secs();

        let response = data::UpResponse {
            tailscale: tailnet_status,
            success,
            lab_info: lab_info.clone(),
            total_time_secs: total_time,
            phases_completed: phases_completed.clone(),
            summary: summary.clone(),
            nodes: node_info_list,
            errors: errors.clone(),
            ssh_config: ssh_files.ssh_config,
            known_hosts: ssh_files.known_hosts,
            ssh_private_key,
            clusters,
            lab_ca: lab_ca.pair.cert.clone(),
            vaults,
            keycloaks,
        };

        tracing::info!(
            lab_id = %lab_id,
            lab_name = %manifest.name,
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
            phases_completed = phases_completed.len(),
            "Lab creation completed"
        );

        Ok(response)
    }; // end resource_creation async block

    let op_attrs = &[KeyValue::new("operation.type", "up")];

    match resource_creation.await {
        Ok(response) => {
            state
                .metrics
                .operation_duration
                .record(start_time.elapsed().as_secs_f64(), op_attrs);
            Ok(response)
        }
        Err(e) => {
            state
                .metrics
                .operation_duration
                .record(start_time.elapsed().as_secs_f64(), op_attrs);

            roll_back_failed_up(state, lab_id, &operation, &progress, &e).await;
            Err(e)
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
