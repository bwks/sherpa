//! Phases 10 and 11: creating the lab's VM and unikernel domains.

use std::sync::Arc;

use anyhow::{Result, bail};
use tokio::task::JoinHandle;
use tracing::instrument;

use super::context::LabBuild;
use crate::services::node_ops;

use shared::data::StatusKind;

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
            tokio::task::spawn(async move { node_ops::create_vm(conn, domain, &progress).await })
        })
        .collect();
    join_all_tasks(tasks, "VM creation").await?;
    let _ = progress.send_status("All VMs created successfully".to_string(), StatusKind::Done);
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
            tokio::task::spawn(
                async move { node_ops::create_unikernel(conn, domain, &progress).await },
            )
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
async fn join_all_tasks(tasks: Vec<JoinHandle<Result<()>>>, what: &str) -> Result<()> {
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
    bail!("{what} failed: {}", failures.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[tokio::test]
    async fn a_failed_task_is_reported_only_after_every_task_ended() {
        let slow_finished = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&slow_finished);
        let tasks = vec![
            tokio::spawn(async { Err(anyhow!("domain r1 failed")) }),
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(200)).await;
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
        let message = format!("{error:#}");
        assert!(
            message.contains("r1") && message.contains("r3"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn tasks_that_all_succeed_pass() {
        let tasks = vec![
            tokio::spawn(async { Ok(()) }),
            tokio::spawn(async { Ok(()) }),
        ];
        join_all_tasks(tasks, "VM creation").await.unwrap();
    }
}
