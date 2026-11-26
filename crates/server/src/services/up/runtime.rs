//! Phases 10 and 11: creating the lab's VM, node VM and unikernel domains.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use tokio::task::{self, JoinHandle};
use tracing::instrument;

use super::context::LabBuild;
use super::links::P2pContainerVeth;
use super::readiness::container_spec;
use crate::services::error::ServiceError;
use crate::services::node_ops::{self, ContainerSettings};
use crate::services::node_vm::{self, NodeVmCreate, NodeVmHost, NodeVmStart};

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

/// Create the VM of every container node that runs in one, all at once, and wait for all of
/// them. Each VM has its node's link ends as taps, so its links are attached with the VMs'
/// (phase 11). Returns the watch on each VM's start, by node name.
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_node_vms(
    build: &LabBuild<'_>,
    nodes: &NodeVmNodes<'_>,
) -> Result<HashMap<String, NodeVmStart>> {
    let progress = build.progress;
    let mut names = Vec::new();
    let mut tasks = Vec::new();
    for container in nodes.container_nodes {
        if !container.model_kind.runs_in_node_vm() {
            continue;
        }
        let create = node_vm_create(build, nodes, container)?;
        let host = nodes.host.clone();
        names.push(container.name.clone());
        tasks.push(task::spawn(async move {
            node_vm::create_node_vm(&host, create).await
        }));
    }
    if tasks.is_empty() {
        return Ok(HashMap::new());
    }
    let _ = progress.send_status(
        format!("Creating {} node VMs in parallel", tasks.len()),
        StatusKind::Progress,
    );
    let starts = join_all_tasks(tasks, "Node VM creation").await?;
    let _ = progress.send_status(
        "All node VMs created successfully".to_string(),
        StatusKind::Done,
    );
    Ok(names.into_iter().zip(starts).collect())
}

/// What the lab's node VMs are created from besides the build.
pub(super) struct NodeVmNodes<'a> {
    pub(super) host: NodeVmHost,
    pub(super) container_nodes: &'a [topology::NodeExpanded],
    pub(super) lab_node_data: &'a [data::LabNodeData],
    pub(super) container_settings: &'a HashMap<String, ContainerSettings>,
    pub(super) secret_mounts: &'a HashMap<String, data::ContainerMount>,
    pub(super) veths: &'a [P2pContainerVeth],
    pub(super) mgmt_net: &'a data::SherpaNetwork,
    pub(super) dns: &'a data::Dns,
}

/// What container node `container`'s VM is created from: its container, its recorded
/// resources, its link ends and its management addresses.
///
/// # Errors
///
/// Fails when the node has no record, recorded resources, settings or management address.
fn node_vm_create(
    build: &LabBuild<'_>,
    nodes: &NodeVmNodes<'_>,
    container: &topology::NodeExpanded,
) -> Result<NodeVmCreate> {
    let name = &container.name;
    let node = nodes
        .lab_node_data
        .iter()
        .find(|node| node.name == *name)
        .with_context(|| format!("Node '{name}' has no record"))?;
    Ok(NodeVmCreate {
        lab_id: build.lab_id.to_owned(),
        lab_dir: build.lab_dir.to_owned(),
        node_name: name.clone(),
        node_index: node.index,
        model_kind: node.model_kind,
        spec: container_spec(
            build.lab_id,
            container,
            nodes.container_settings,
            nodes.secret_mounts,
        )?,
        dns: nodes.dns.clone(),
        memory_mib: node
            .record
            .memory
            .map(u64::from)
            .with_context(|| format!("Node '{name}' has no memory recorded"))?,
        vcpus: node
            .record
            .cpu_count
            .with_context(|| format!("Node '{name}' has no vCPUs recorded"))?,
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
