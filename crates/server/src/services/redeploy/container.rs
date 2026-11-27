//! Stages 4 and 5 for a container: its configuration, its data networks or P2p veths, and
//! starting it; then the services a container node runs.

use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::time;
use tracing::instrument;

use super::context::Redeploy;
use super::ready::{ReadyWait, ready_wait, wait_until_ready};
use crate::konst::{SHERPA_MANAGEMENT_NETWORK_NAME, SHERPA_SECRETS_DIR};
use crate::services::container_networks;
use crate::services::error::{SandboxRefusal, ServiceError};
use crate::services::keycloak;
use crate::services::lab_operations::LabOperation;
use crate::services::node_lifecycle::{self, query_runtimes};
use crate::services::node_ops::{self, ContainerSettings, ContainerZtpResult, LabZtp};
use crate::services::node_secrets::secrets_for_redeploy;
use crate::services::p2p_links::{
    ContainerNode, ContainerVeth, KernelHost, LabLinks, RecreatedEnd, container_veths,
    data_interface_indices, follow_started_links, node_bridge_interfaces, node_links,
    reattach_node_links, report_links,
};
use crate::services::sandbox::{self, SandboxCreate, SandboxHost};
use crate::services::ssh_config;
use crate::services::talos;
use crate::services::vault;
use crate::tls::lab_ca::NodeCertPaths;

use container::ContainerSpec;
use shared::data;
use shared::data::{LinkReport, OperationEvent, RecordId, StatusKind};
use shared::konst::{CONTAINER_SECRETS_DIR, SHERPA_LABS_PATH};
use shared::util;

/// A container's data interfaces, and the lab's links they were planned from.
struct DataInterfaces {
    lab_links: LabLinks,
    networks: Vec<data::ContainerNetworkAttachment>,
    veths: Vec<ContainerVeth>,
}

/// Configure the container again, recreate its data networks or P2p veths, start it, re-attach
/// its links, and restore what its services keep (stages 4 and 5). With a stored ready port, it
/// is then recorded running only once that port answers, as a VM is. Returns its links'
/// states.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn redeploy_container(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<Vec<LinkReport>> {
    // Read before the node's ZTP takes a custom config out of it.
    let initialises_vault = vault::sherpa_initialises_vault(
        redeploy.target_node.model_kind.model(),
        redeploy.target_node.ztp.config.is_some(),
    );
    let ztp_result = configure_container(redeploy, cert_paths)?;

    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let _ = redeploy.progress.send_status(
        format!("Recreating networks and starting container: {node_name}"),
        StatusKind::Progress,
    );

    let container_name = format!("{node_name}-{lab_id}");

    let DataInterfaces {
        lab_links,
        networks: additional_networks,
        veths: p2p_veths,
    } = plan_data_interfaces(redeploy).await?;
    let db = &redeploy.state.db;

    let has_disabled = additional_networks.iter().any(|n| n.admin_down);
    let is_running = node_ops::start_container_node(
        &redeploy.state.docker,
        &redeploy.state.qemu,
        ContainerSpec {
            additional_networks,
            memory_mib: redeploy.db_node.memory.map(u64::from),
            cpu_count: redeploy.db_node.cpu_count,
            ..redeployed_container_spec(redeploy, ztp_result.settings)?
        },
        redeploy.target_node.model_kind,
        redeploy.progress,
    )
    .await?;

    if !is_running {
        bail!("Container {container_name} is not in running state after redeploy");
    }
    node_lifecycle::record(
        db,
        redeploy.operation,
        &redeploy.node_record_id,
        OperationEvent::Launched,
    )
    .await?;

    // Take the carrier from disabled interfaces, as for VMs.
    if has_disabled {
        let isolated =
            node_ops::node_isolated_network_data(node_name, redeploy.db_node.index, lab_id);
        network::set_link_state(&isolated.bridge_name, false).await?;
    }

    let link_reports = if p2p_veths.is_empty() {
        Vec::new()
    } else {
        attach_links(
            redeploy,
            &lab_links,
            RecreatedEnd::Container {
                container: container_name,
                veths: p2p_veths,
            },
        )
        .await
    };

    match ready_wait(&redeploy.db_node.ready_check) {
        ReadyWait::Port(_) => {
            restore_node_services(redeploy, initialises_vault).await?;
            wait_until_ready(redeploy).await?;
        }
        ReadyWait::Skipped | ReadyWait::Running => {
            ready_after_restore(
                db,
                redeploy.operation,
                &redeploy.node_record_id,
                restore_node_services(redeploy, initialises_vault),
            )
            .await?;
        }
    }
    Ok(link_reports)
}

/// Configure a container that runs in a sandbox again, create its VM with its link ends as
/// taps, wait for the VM to start its container, re-attach its links, wait for it as
/// `redeploy_container` does (stages 4 and 5), and record the host key of the new VM's sshd,
/// returning its links' states.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
pub(super) async fn redeploy_sandbox(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<Vec<LinkReport>> {
    let ztp_result = configure_container(redeploy, cert_paths)?;
    let node_name = redeploy.node_name.clone();
    let _ = redeploy.progress.send_status(
        format!("Creating the VM of node: {node_name}"),
        StatusKind::Progress,
    );
    let db = &redeploy.state.db;
    let lab_links = LabLinks::load(db, &redeploy.lab_id, &redeploy.lab_record_id).await?;
    let mut watch = sandbox::create_sandbox(
        &SandboxHost {
            docker: Arc::clone(&redeploy.state.docker),
            qemu: Arc::clone(&redeploy.state.qemu),
            qemu_bin: redeploy.state.config.qemu_bin.clone(),
            base: redeploy.sandbox_base.clone(),
            disk_locks: redeploy.state.sandbox_disk_locks.clone(),
            progress: redeploy.progress.clone(),
        },
        SandboxCreate {
            lab_id: redeploy.lab_id.clone(),
            lab_dir: redeploy.lab_dir.clone(),
            node_name: node_name.clone(),
            node_index: redeploy.db_node.index,
            model_kind: redeploy.target_node.model_kind,
            spec: redeployed_container_spec(redeploy, ztp_result.settings)?,
            dns: redeploy.dns.clone(),
            memory: redeploy
                .db_node
                .memory
                .context("The node has no memory recorded")?,
            vcpus: redeploy
                .db_node
                .cpu_count
                .context("The node has no vCPUs recorded")?,
            tuning: sandbox::sandbox_tuning(&redeploy.target_node),
            veths: sandbox_link_ends(redeploy, &lab_links).await?,
            mgmt_net: redeploy.mgmt_net.clone(),
            ipv4: redeploy.ipv4,
            ipv6: redeploy.target_node.ipv6_address,
            own_address: redeploy.vm_sandbox.clone().ok_or_else(|| {
                ServiceError::Sandbox(SandboxRefusal::AddressMissing {
                    node: node_name.clone(),
                })
            })?,
            ssh_keys: redeploy.owner_account.ssh_keys.clone(),
        },
    )
    .await?;
    let timeout = redeploy.ready_timeout();
    time::timeout(timeout, watch.wait_started())
        .await
        .with_context(|| {
            format!(
                "Sandbox {node_name} did not start its container within {} seconds",
                timeout.as_secs()
            )
        })??;
    node_lifecycle::record(
        db,
        redeploy.operation,
        &redeploy.node_record_id,
        OperationEvent::Launched,
    )
    .await?;
    let link_reports = attach_links(redeploy, &lab_links, RecreatedEnd::Domain).await;
    match ready_wait(&redeploy.db_node.ready_check) {
        ReadyWait::Port(_) => wait_until_ready(redeploy).await?,
        ReadyWait::Skipped | ReadyWait::Running => {
            node_lifecycle::record(
                db,
                redeploy.operation,
                &redeploy.node_record_id,
                OperationEvent::Ready,
            )
            .await?;
        }
    }
    if let Some(sandbox) = &redeploy.vm_sandbox
        && let Err(error) = ssh_config::capture_sandbox_host_key(db, sandbox).await
    {
        tracing::warn!(error = %format!("{error:#}"), "Failed to record the sandbox host key");
    }
    Ok(link_reports)
}

/// The container a redeployed node runs, from its generated `settings` with its secret files,
/// replaced, removed or kept, mounted from RAM.
///
/// # Errors
///
/// Fails when the secret files cannot be written.
fn redeployed_container_spec(
    redeploy: &Redeploy<'_>,
    mut settings: ContainerSettings,
) -> Result<ContainerSpec> {
    let node_name = redeploy.node_name.as_str();
    if let Some(dir) = secrets_for_redeploy(
        Path::new(SHERPA_SECRETS_DIR),
        &redeploy.lab_id,
        node_name,
        redeploy.secrets.as_deref(),
    )
    .with_context(|| format!("Cannot write secret files for node '{node_name}'"))?
    {
        settings.volumes.push(data::ContainerMount::read_only(
            dir.display().to_string(),
            CONTAINER_SECRETS_DIR,
        ));
    }
    Ok(node_ops::container_spec(
        &redeploy.lab_id,
        node_name,
        &redeploy.node_image.version,
        settings,
        data::ContainerNetworkAttachment {
            name: format!("{SHERPA_MANAGEMENT_NETWORK_NAME}-{}", redeploy.lab_id),
            ipv4_address: Some(redeploy.ipv4.to_string()),
            ipv6_address: None,
            linux_interface_name: None,
            admin_down: false,
        },
    ))
}

/// The link ends of a sandbox being redeployed: a tap for each of its data interfaces when it
/// is on a link or bridge, else none.
async fn sandbox_link_ends(
    redeploy: &Redeploy<'_>,
    lab_links: &LabLinks,
) -> Result<Vec<ContainerVeth>> {
    let links = node_links(lab_links.links(), &redeploy.node_record_id);
    let bridge_interfaces = node_bridge_interfaces(
        &db::list_bridges(&redeploy.state.db, &redeploy.lab_record_id).await?,
        &redeploy.node_record_id,
    );
    if links.is_empty() && bridge_interfaces.is_empty() {
        return Ok(vec![]);
    }
    container_veths(
        &ContainerNode {
            lab_id: &redeploy.lab_id,
            name: &redeploy.node_name,
            index: redeploy.db_node.index,
            model_kind: redeploy.target_node.model_kind,
            record: &redeploy.node_record_id,
        },
        &links,
        &bridge_interfaces,
        data_interface_indices(
            &redeploy.node_image,
            redeploy.data_interface_count,
            &redeploy.node_name,
        )?,
    )
}

/// Make the redeployed node's link ends `end`, attach its links, and report each link's state.
async fn attach_links(
    redeploy: &Redeploy<'_>,
    lab_links: &LabLinks,
    end: RecreatedEnd,
) -> Vec<LinkReport> {
    let host = KernelHost::new(&redeploy.state.docker, &redeploy.state.qemu);
    let outcomes = reattach_node_links(&host, lab_links, &redeploy.node_record_id, end).await;
    let reports = report_links(
        &host,
        lab_links,
        &redeploy.node_record_id,
        Some(redeploy.progress),
        &redeploy.state.link_states,
        outcomes,
    )
    .await;
    // Re-attach takes a paused peer as up; decide the links as the next scan would.
    follow_started_links(
        &host,
        lab_links,
        &HashSet::from([redeploy.node_record_id.clone()]),
        &query_runtimes(&redeploy.state.docker, &redeploy.state.qemu).await,
        &redeploy.state.link_states,
        &redeploy.state.unreadable_ends,
    )
    .await;
    reports
}

/// Plan a container's data interfaces: Docker networks, or veths when it is on a link or bridge.
async fn plan_data_interfaces(redeploy: &Redeploy<'_>) -> Result<DataInterfaces> {
    let db = &redeploy.state.db;
    let lab_links = LabLinks::load(db, &redeploy.lab_id, &redeploy.lab_record_id).await?;
    let links = node_links(lab_links.links(), &redeploy.node_record_id);
    let db_bridges = db::list_bridges(db, &redeploy.lab_record_id).await?;
    let bridge_interfaces = node_bridge_interfaces(&db_bridges, &redeploy.node_record_id);
    let data_indices = data_interface_indices(
        &redeploy.node_image,
        redeploy.data_interface_count,
        &redeploy.node_name,
    )?;
    let (networks, veths) = if links.is_empty() && bridge_interfaces.is_empty() {
        let networks = create_data_networks(redeploy, data_indices).await?;
        (networks, vec![])
    } else {
        let node = ContainerNode {
            lab_id: &redeploy.lab_id,
            name: &redeploy.node_name,
            index: redeploy.db_node.index,
            model_kind: redeploy.target_node.model_kind,
            record: &redeploy.node_record_id,
        };
        let veths = container_veths(&node, &links, &bridge_interfaces, data_indices)?;
        (vec![], veths)
    };
    Ok(DataInterfaces {
        lab_links,
        networks,
        veths,
    })
}

/// Restore what the node's services keep, then record the node ready. A container whose
/// services were not restored is never recorded running: it stays `Starting` until the
/// redeploy records it failed.
async fn ready_after_restore(
    db: &db::Database,
    operation: &LabOperation,
    node_id: &RecordId,
    restore: impl Future<Output = Result<()>>,
) -> Result<()> {
    restore.await?;
    node_lifecycle::record(db, operation, node_id, OperationEvent::Ready).await?;
    Ok(())
}

/// Generate the container's configuration (stage 4), with the image stage 1 resolved and
/// checked.
fn configure_container(
    redeploy: &mut Redeploy<'_>,
    cert_paths: &NodeCertPaths,
) -> Result<ContainerZtpResult> {
    node_ops::generate_container_ztp(
        &mut redeploy.target_node,
        &redeploy.node_image,
        &LabZtp {
            lab_id: &redeploy.lab_id,
            lab_dir: &redeploy.lab_dir,
            images_dir: &redeploy.state.config.images_dir,
            dns: &redeploy.dns,
            mgmt_net: &redeploy.mgmt_net,
            progress: redeploy.progress,
            owner_account: &redeploy.owner_account,
        },
        redeploy.ipv4,
        Some(cert_paths),
    )
}

/// Create the disabled Docker macvlan networks of a container on no link and no bridge.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
async fn create_data_networks(
    redeploy: &Redeploy<'_>,
    data_indices: RangeInclusive<u8>,
) -> Result<Vec<data::ContainerNetworkAttachment>> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let model_kind = redeploy.target_node.model_kind;
    let isolated = node_ops::node_isolated_network_data(node_name, redeploy.db_node.index, lab_id);
    let mut attachments = vec![];

    for idx in data_indices {
        let iface_name = util::interface_from_idx(model_kind, idx)?;
        container::create_docker_macvlan_bridge_network(
            &redeploy.state.docker,
            &isolated.bridge_name,
            &container_networks::isolated_network_name(node_name, idx, lab_id),
        )
        .await?;
        attachments.push(container_networks::data_attachment(
            node_name,
            model_kind,
            &iface_name,
            idx,
            lab_id,
        )?);
    }
    Ok(attachments)
}

/// A redeployed Vault starts with empty storage: initialise and unseal it again, clear what was
/// kept of it being sealed, and let the clusters that use it log in again. The client's `sherpa download` after the redeploy writes
/// its new credentials. A redeployed Keycloak starts with an empty database: make the realm
/// again, with the users' saved passwords.
#[instrument(skip_all, fields(lab_id = %redeploy.lab_id, node_name = %redeploy.node_name), level = "debug")]
async fn restore_node_services(redeploy: &Redeploy<'_>, initialises_vault: bool) -> Result<()> {
    let lab_id = redeploy.lab_id.as_str();
    let node_name = redeploy.node_name.as_str();
    let ca_cert = &redeploy.lab_ca.pair.cert;
    if initialises_vault {
        let info = vault::ensure_vault_unsealed(
            lab_id,
            node_name,
            redeploy.ipv4,
            ca_cert,
            redeploy.ready_timeout(),
            redeploy.progress,
        )
        .await?;
        vault::record_seal_outcome(
            &redeploy.state.vault_seals,
            lab_id,
            node_name,
            &vault::SealOutcome::Unsealed,
        );
        talos::grant_vault_access(
            &talos::load_clusters(
                &redeploy.state.db,
                Path::new(SHERPA_LABS_PATH),
                &redeploy.lab_record_id,
                lab_id,
            )
            .await?,
            node_name,
            redeploy.ipv4,
            ca_cert,
            &info.root_token,
        )
        .await?;
    }
    if redeploy.target_node.model_kind.model() == data::NodeModel::KeycloakServer {
        keycloak::ensure_keycloak_realm(
            lab_id,
            node_name,
            redeploy.ipv4,
            ca_cert,
            redeploy.ready_timeout(),
            redeploy.progress,
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io;

    use shared::data::NodeState;

    use super::*;
    use crate::services::lab_operations::{Operation, begin};
    use crate::services::node_lifecycle::tests::lab_of;

    async fn launched_container(lab_id: &str) -> (db::Database, RecordId) {
        let (db, _, nodes) = lab_of(lab_id, 1).await;
        let operation = begin(lab_id, Operation::Redeploy).await;
        node_lifecycle::record(&db, &operation, &nodes[0], OperationEvent::Launched)
            .await
            .unwrap();
        (db, nodes[0].clone())
    }

    #[tokio::test]
    async fn cluster_redeploy_004_a_container_is_not_running_before_its_services_are_restored() {
        let (db, node) = launched_container("rd-rest1").await;
        let operation = begin("rd-rest1", Operation::Redeploy).await;
        let restore = async {
            let state = db::get_node(&db, node.clone()).await?.state;
            assert_eq!(state, NodeState::Starting);
            Err(io::Error::from(io::ErrorKind::TimedOut).into())
        };

        let error = ready_after_restore(&db, &operation, &node, restore)
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<io::Error>().map(io::Error::kind),
            Some(io::ErrorKind::TimedOut)
        );
        assert_eq!(
            db::get_node(&db, node).await.unwrap().state,
            NodeState::Starting
        );
    }

    #[tokio::test]
    async fn cluster_redeploy_005_a_container_whose_services_are_restored_is_running() {
        let (db, node) = launched_container("rd-rest2").await;
        let operation = begin("rd-rest2", Operation::Redeploy).await;

        ready_after_restore(&db, &operation, &node, async { Ok(()) })
            .await
            .unwrap();
        assert_eq!(
            db::get_node(&db, node).await.unwrap().state,
            NodeState::Running
        );
    }
}
