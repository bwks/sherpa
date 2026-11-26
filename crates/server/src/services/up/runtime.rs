//! Phases 10 and 11: creating the lab's VM, node VM and unikernel domains.

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use futures::future::join_all;
use tokio::task::{self, JoinHandle};
use tracing::instrument;

use super::context::LabBuild;
use super::links::P2pContainerVeth;
use crate::services::error::ServiceError;
use crate::services::node_ops;
use crate::services::node_vm::{self, NodeVmCreate};

use shared::data::{self as data, StatusKind};

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
/// (phase 11).
#[instrument(skip_all, fields(lab_id = %build.lab_id))]
pub(super) async fn create_node_vms(
    build: &LabBuild<'_>,
    container_nodes: &[topology::NodeExpanded],
    lab_node_data: &[data::LabNodeData],
    veths: &[P2pContainerVeth],
) -> Result<()> {
    let nodes: Vec<&data::LabNodeData> = lab_node_data
        .iter()
        .filter(|node| node.model_kind.runs_in_node_vm())
        .filter(|node| container_nodes.iter().any(|c| c.name == node.name))
        .collect();
    if nodes.is_empty() {
        return Ok(());
    }
    let _ = build.progress.send_status(
        format!("Creating {} node VMs in parallel", nodes.len()),
        StatusKind::Progress,
    );
    let creates = nodes
        .iter()
        .map(|node| -> Result<NodeVmCreate<'_>> {
            let container = container_nodes
                .iter()
                .find(|container| container.name == node.name)
                .with_context(|| format!("Node '{}' is not a container", node.name))?;
            let (Some(image), Some(version)) = (&container.image, &container.version) else {
                bail!("Node '{}' has no image and version", node.name);
            };
            Ok(NodeVmCreate {
                lab_id: build.lab_id,
                lab_dir: build.lab_dir,
                images_dir: &build.config.images_dir,
                node_name: &node.name,
                node_index: node.index,
                model_kind: node.model_kind,
                image: container::image_reference(image, version),
                memory_mib: node
                    .record
                    .memory
                    .map(u64::from)
                    .with_context(|| format!("Node '{}' has no memory recorded", node.name))?,
                vcpus: node
                    .record
                    .cpu_count
                    .with_context(|| format!("Node '{}' has no vCPUs recorded", node.name))?,
                veths: veths
                    .iter()
                    .filter(|veth| veth.node_name == node.name)
                    .map(|veth| &veth.veth)
                    .collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let results = join_all(
        creates
            .iter()
            .map(|create| node_vm::create_node_vm(build.docker_conn, build.qemu, create)),
    )
    .await;
    for (create, result) in creates.iter().zip(results) {
        result.with_context(|| format!("Failed to create the VM of node {}", create.node_name))?;
        let _ = build.progress.send_status(
            format!("Created node VM: {}", create.node_name),
            StatusKind::Done,
        );
    }
    Ok(())
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

/// Wait for every task, then report the failures together.
///
/// Stopping at the first failure would drop the other handles, which detaches their tasks:
/// they would go on creating domains while the rollback that follows removes the lab.
async fn join_all_tasks(tasks: Vec<JoinHandle<Result<()>>>, what: &'static str) -> Result<()> {
    let mut failures = Vec::new();
    for task in tasks {
        match task.await {
            Ok(Ok(())) => tracing::debug!("{what} task finished"),
            Ok(Err(error)) => failures.push(format!("{error:#}")),
            Err(error) => failures.push(format!("task failed: {error}")),
        }
    }
    if failures.is_empty() {
        return Ok(());
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
