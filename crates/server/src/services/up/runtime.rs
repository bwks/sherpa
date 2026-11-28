//! Phases 10 and 11: creating the lab's VM, sandbox and unikernel domains.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::task::{self, JoinHandle};
use tracing::instrument;

use super::context::LabBuild;
use super::links::P2pContainerVeth;
use super::readiness::container_spec;
use super::ztp::SandboxRecord;
use crate::services::error::ServiceError;
use crate::services::node_ops::{self, ContainerSettings};
use crate::services::sandbox::{self, SandboxCreate, SandboxHost, StatusWatch};

use shared::data::{self, StatusKind};

/// Create every VM domain at once, and wait for all of them (phase 11).
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_vms(
    build: &LabBuild<'_>,
    domains: Vec<template::DomainTemplate>,
) -> Result<()> {
    let progress = build.progress;
    if domains.is_empty() {
        let _ = progress.send_status("No VMs to create".to_string(), StatusKind::Info);
        return Ok(());
    }
    let _ = progress.send_status(
        format!("Creating {} VMs in parallel", domains.len()),
        StatusKind::Progress,
    );
    let tasks = domains
        .into_iter()
        .map(|domain| {
            let conn = Arc::clone(build.qemu_conn);
            let progress = progress.clone();
            task::spawn(async move { node_ops::create_vm(conn, domain, &progress).await })
        })
        .collect();
    join_all_tasks(tasks, "VM creation").await?;
    let _ = progress.send_status("All VMs created successfully".to_string(), StatusKind::Done);
    Ok(())
}

/// Create the VM of every container node that runs in one, all at once once every node's create
/// is built, wait for all of them, and return the watch on each VM's status by node name
/// (phase 11, its links attached with the VMs' as its link ends are taps).
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_sandboxes(
    build: &LabBuild<'_>,
    nodes: &SandboxNodes<'_>,
) -> Result<HashMap<String, StatusWatch>> {
    let progress = build.progress;
    let creates = nodes
        .container_nodes
        .iter()
        .filter(|container| container.model_kind.runs_in_sandbox())
        .map(|container| sandbox_create(build, nodes, container))
        .collect::<Result<Vec<_>>>()?;
    if creates.is_empty() {
        return Ok(HashMap::new());
    }
    let names: Vec<String> = creates
        .iter()
        .map(|create| create.node_name.clone())
        .collect();
    let tasks: Vec<_> = creates
        .into_iter()
        .map(|create| {
            let host = nodes.host.clone();
            task::spawn(async move { sandbox::create_sandbox(&host, create).await })
        })
        .collect();
    let _ = progress.send_status(
        format!("Creating {} sandboxes in parallel", tasks.len()),
        StatusKind::Progress,
    );
    let starts = join_all_tasks(tasks, "Sandbox creation").await?;
    let _ = progress.send_status(
        "All sandboxes created successfully".to_string(),
        StatusKind::Done,
    );
    Ok(names.into_iter().zip(starts).collect())
}

/// What the lab's sandboxes are created from besides the build.
pub(super) struct SandboxNodes<'a> {
    pub(super) host: SandboxHost,
    pub(super) container_nodes: &'a [topology::NodeExpanded],
    pub(super) lab_node_data: &'a [data::LabNodeData],
    pub(super) container_settings: &'a HashMap<String, ContainerSettings>,
    pub(super) secret_mounts: &'a HashMap<String, data::ContainerMount>,
    pub(super) veths: &'a [P2pContainerVeth],
    pub(super) mgmt_net: &'a data::SherpaNetwork,
    pub(super) dns: &'a data::Dns,
    pub(super) sandbox_records: &'a [SandboxRecord],
}

/// What container node `container`'s VM is created from: its container, its recorded
/// resources, its link ends, its management addresses and its sandbox's own.
///
/// # Errors
///
/// Fails when the node has no record, recorded resources, settings, management address or
/// `vm_sandbox` row.
fn sandbox_create(
    build: &LabBuild<'_>,
    nodes: &SandboxNodes<'_>,
    container: &topology::NodeExpanded,
) -> Result<SandboxCreate> {
    let name = &container.name;
    let node = nodes
        .lab_node_data
        .iter()
        .find(|node| node.name == *name)
        .with_context(|| format!("Node '{name}' has no record"))?;
    Ok(SandboxCreate {
        lab_id: build.lab_id.to_owned(),
        lab_dir: build.lab_dir.to_owned(),
        node_name: name.clone(),
        node_index: node.index,
        model_kind: node.model_kind,
        version: container
            .version
            .clone()
            .with_context(|| format!("Node '{name}' has no image version"))?,
        spec: container_spec(
            build.lab_id,
            container,
            nodes.container_settings,
            nodes.secret_mounts,
        )?,
        dns: nodes.dns.clone(),
        memory: node
            .record
            .memory
            .with_context(|| format!("Node '{name}' has no memory recorded"))?,
        vcpus: node
            .record
            .cpu_count
            .with_context(|| format!("Node '{name}' has no vCPUs recorded"))?,
        tuning: sandbox::sandbox_tuning(container),
        veths: nodes
            .veths
            .iter()
            .filter(|veth| veth.node_name == *name)
            .map(|veth| veth.veth.clone())
            .collect(),
        mgmt_net: nodes.mgmt_net.clone(),
        ipv4: container
            .ipv4_address
            .with_context(|| format!("Node '{name}' has no management address"))?,
        ipv6: container.ipv6_address,
        own_address: nodes
            .sandbox_records
            .iter()
            .find(|sandbox| sandbox.node_name == *name)
            .map(|sandbox| sandbox.record.clone())
            .with_context(|| format!("The sandbox of node '{name}' has no management address"))?,
    })
}

/// Create every unikernel domain at once, and wait for all of them.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_unikernels(
    build: &LabBuild<'_>,
    domains: Vec<template::UnikernelDomainTemplate>,
) -> Result<()> {
    if domains.is_empty() {
        return Ok(());
    }
    let progress = build.progress;
    let _ = progress.send_status(
        format!("Creating {} unikernels in parallel", domains.len()),
        StatusKind::Progress,
    );
    let tasks = domains
        .into_iter()
        .map(|domain| {
            let conn = Arc::clone(build.qemu_conn);
            let progress = progress.clone();
            task::spawn(async move { node_ops::create_unikernel(conn, domain, &progress).await })
        })
        .collect();
    join_all_tasks(tasks, "Unikernel creation").await?;
    let _ = progress.send_status(
        "All unikernels created successfully".to_string(),
        StatusKind::Done,
    );
    Ok(())
}

/// Wait for every task, then report the failures together, or return each task's result in
/// order.
///
/// Stopping at the first failure would drop the other handles, which detaches their tasks:
/// they would go on creating domains while the rollback that follows removes the lab.
async fn join_all_tasks<T>(
    tasks: Vec<JoinHandle<Result<T>>>,
    what: &'static str,
) -> Result<Vec<T>> {
    let mut results = Vec::new();
    let mut failures = Vec::new();
    for task in tasks {
        match task.await {
            Ok(Ok(result)) => {
                tracing::debug!("{what} task finished");
                results.push(result);
            }
            Ok(Err(error)) => failures.push(format!("{error:#}")),
            Err(error) => failures.push(format!("task failed: {error}")),
        }
    }
    if failures.is_empty() {
        return Ok(results);
    }
    bail!(ServiceError::TasksFailed { what, failures })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::time;

    #[tokio::test]
    async fn a_failed_task_is_reported_only_after_every_task_ended() {
        let slow_finished = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&slow_finished);
        let tasks = vec![
            tokio::spawn(async { Err(anyhow!("domain r1 failed")) }),
            tokio::spawn(async move {
                time::sleep(Duration::from_millis(200)).await;
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }),
            tokio::spawn(async { Err(anyhow!("domain r3 failed")) }),
        ];

        let error = join_all_tasks(tasks, "VM creation")
            .await
            .expect_err("failures hidden");

        assert!(
            slow_finished.load(Ordering::SeqCst),
            "returned before the slow task ended"
        );
        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::TasksFailed { what: "VM creation", failures })
                    if failures.len() == 2
            ),
            "{error:#}"
        );
    }
}
