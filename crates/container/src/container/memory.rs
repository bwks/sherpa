use anyhow::{Context, Result};
use bollard::Docker;
use bollard::models::{ContainerInspectResponse, ContainerStateStatusEnum};
use bollard::query_parameters::InspectContainerOptions;
use tracing::instrument;

use super::list::list_containers;
use crate::status::is_missing;

/// A container's hard memory limit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContainerMemory {
    /// The container's name, without Docker's leading `/`.
    pub name: String,
    /// Whether it is running, paused or restarting, so its memory is in use.
    pub active: bool,
    /// Its memory limit, in bytes.
    pub limit_bytes: u64,
}

/// The memory limits of every container that has one, running or not. A container removed
/// while the list is read is left out.
///
/// # Errors
///
/// Fails when Docker cannot list or inspect the containers.
#[instrument(skip(docker), level = "debug")]
pub async fn list_container_memory(docker: &Docker) -> Result<Vec<ContainerMemory>> {
    let mut limits = Vec::new();
    for summary in list_containers(docker).await? {
        let Some(id) = summary.id else {
            continue;
        };
        match docker
            .inspect_container(&id, Some(InspectContainerOptions { size: false }))
            .await
        {
            Ok(details) => limits.extend(memory_limit(&details)),
            Err(error) if is_missing(&error) => {}
            Err(error) => {
                return Err(error).with_context(|| format!("failed to inspect container {id}"));
            }
        }
    }
    Ok(limits)
}

/// The memory limit of an inspected container, or `None` when it has no limit.
fn memory_limit(details: &ContainerInspectResponse) -> Option<ContainerMemory> {
    let limit_bytes = details
        .host_config
        .as_ref()
        .and_then(|host_config| host_config.memory)
        .and_then(|memory| u64::try_from(memory).ok())
        .filter(|memory| *memory > 0)?;
    let name = details.name.as_deref()?.trim_start_matches('/').to_string();
    let active = matches!(
        details.state.as_ref().and_then(|state| state.status),
        Some(
            ContainerStateStatusEnum::RUNNING
                | ContainerStateStatusEnum::PAUSED
                | ContainerStateStatusEnum::RESTARTING
        )
    );
    Some(ContainerMemory {
        name,
        active,
        limit_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerState, HostConfig};

    fn inspected(
        memory: Option<i64>,
        status: ContainerStateStatusEnum,
    ) -> ContainerInspectResponse {
        ContainerInspectResponse {
            name: Some("/db-lab1".to_string()),
            state: Some(ContainerState {
                status: Some(status),
                ..Default::default()
            }),
            host_config: Some(HostConfig {
                memory,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_limited_container_reports_its_name_limit_and_activity() {
        assert_eq!(
            memory_limit(&inspected(
                Some(1_073_741_824),
                ContainerStateStatusEnum::RUNNING
            )),
            Some(ContainerMemory {
                name: "db-lab1".to_string(),
                active: true,
                limit_bytes: 1_073_741_824,
            })
        );
        let paused = memory_limit(&inspected(Some(1), ContainerStateStatusEnum::PAUSED)).unwrap();
        assert!(paused.active);
        let stopped = memory_limit(&inspected(Some(1), ContainerStateStatusEnum::EXITED)).unwrap();
        assert!(!stopped.active);
    }

    #[test]
    fn an_unlimited_container_has_no_limit() {
        // Docker reports no limit as 0.
        assert_eq!(
            memory_limit(&inspected(Some(0), ContainerStateStatusEnum::RUNNING)),
            None
        );
        assert_eq!(
            memory_limit(&inspected(None, ContainerStateStatusEnum::RUNNING)),
            None
        );
    }
}
