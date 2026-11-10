//! Moving nodes through their lifecycle, and the one table that maps libvirt's and Docker's
//! states to `Runtime`.
//!
//! The allowed moves are in `shared::data::NodeState::on_operation` and `on_observation`.
//! Here they are written:
//!
//! - `record` applies an operation's event. It takes the operation's `LabOperation`, so only
//!   code holding the lab's lease can call it.
//! - `observe` applies what the scanner saw. It takes an `ObservedLab`, which the scanner only
//!   gets while no operation holds the lab.
//!
//! Each write is a compare-and-set on the state the node was read in. When another writer moved
//! the node in between, the event is evaluated again against the new state, so it is never
//! applied to a state it was not meant for. A lab's status is derived from its nodes when it is
//! read, so it follows every write, scanner or not.

use anyhow::{Context, Result, bail};
use bollard::secret::ContainerStateStatusEnum;
use shared::data::{DbNode, Halt, NodeState, Observation, OperationEvent, RecordId, Runtime};
use tracing::instrument;
use virt::sys::{
    VIR_DOMAIN_BLOCKED, VIR_DOMAIN_CRASHED, VIR_DOMAIN_NOSTATE, VIR_DOMAIN_PAUSED,
    VIR_DOMAIN_PMSUSPENDED, VIR_DOMAIN_RUNNING, VIR_DOMAIN_SHUTDOWN, VIR_DOMAIN_SHUTOFF,
};

use crate::services::lab_operations::{LabOperation, ObservedLab};

/// How often a write that lost a race to another writer is tried again.
const WRITE_ATTEMPTS: usize = 3;

/// What libvirt's domain state `code` means for a node. `None` is a domain that does not exist.
#[must_use]
pub fn libvirt_runtime(code: Option<u32>) -> Runtime {
    match code {
        None => Runtime::Absent,
        Some(VIR_DOMAIN_RUNNING) => Runtime::Running,
        Some(VIR_DOMAIN_SHUTOFF) => Runtime::Stopped(Halt::Off),
        Some(VIR_DOMAIN_PAUSED) => Runtime::Stopped(Halt::Paused),
        Some(VIR_DOMAIN_CRASHED) => Runtime::Faulted("crashed"),
        Some(VIR_DOMAIN_SHUTDOWN) => Runtime::Transitional("shutting down"),
        Some(VIR_DOMAIN_BLOCKED) => Runtime::Transitional("blocked"),
        Some(VIR_DOMAIN_PMSUSPENDED) => Runtime::Transitional("suspended"),
        Some(VIR_DOMAIN_NOSTATE | _) => Runtime::Transitional("in no state"),
    }
}

/// What a Docker container's status means for a node. `None` is a container that does not
/// exist.
#[must_use]
pub fn container_runtime(status: Option<ContainerStateStatusEnum>) -> Runtime {
    match status {
        None => Runtime::Absent,
        Some(ContainerStateStatusEnum::RUNNING) => Runtime::Running,
        Some(ContainerStateStatusEnum::EXITED | ContainerStateStatusEnum::CREATED) => {
            Runtime::Stopped(Halt::Off)
        }
        Some(ContainerStateStatusEnum::PAUSED) => Runtime::Stopped(Halt::Paused),
        Some(ContainerStateStatusEnum::DEAD) => Runtime::Faulted("dead"),
        Some(ContainerStateStatusEnum::RESTARTING) => Runtime::Transitional("restarting"),
        Some(ContainerStateStatusEnum::REMOVING) => Runtime::Transitional("being removed"),
        Some(ContainerStateStatusEnum::EMPTY) => Runtime::Transitional("in no state"),
    }
}

/// Apply `event`, from the operation holding `_operation`, to node `node_id`, and return the
/// node's new state.
///
/// # Errors
/// When the node's state does not allow the event, the error says why (its source is the
/// `Refused`); also when the node does not exist or the database fails.
#[instrument(skip(db, _operation), level = "debug")]
pub async fn record(
    db: &db::Database,
    _operation: &LabOperation,
    node_id: &RecordId,
    event: OperationEvent,
) -> Result<NodeState> {
    let mut node = db::get_node(db, node_id.clone()).await?;
    for _ in 0..WRITE_ATTEMPTS {
        let to = node.state.on_operation(event)?;
        match write(db, &node, to).await? {
            Some(moved) => return Ok(moved.state),
            None => node = db::get_node(db, node_id.clone()).await?,
        }
    }
    bail!(
        "Node {} kept changing state; {event} not recorded",
        node.name
    )
}

/// Apply what the scanner saw of `node`, read under `_observed`, and return its new state:
/// `None` when another writer moved it since it was read, which leaves it for the next cycle.
///
/// # Errors
/// When the database fails.
#[instrument(skip(db, _observed, node), fields(node_name = %node.name), level = "debug")]
pub async fn observe(
    db: &db::Database,
    _observed: &ObservedLab,
    node: &DbNode,
    observation: Observation,
) -> Result<Option<NodeState>> {
    let to = node.state.on_observation(observation);
    if to != node.state {
        tracing::info!(node = %node.name, from = %node.state, to = %to, "Node state changed");
    }
    Ok(write(db, node, to).await?.map(|moved| moved.state))
}

/// Write state `to` for `node` if it is still in the state it was read in. `None` when another
/// writer moved it first.
async fn write(db: &db::Database, node: &DbNode, to: NodeState) -> Result<Option<DbNode>> {
    if to == node.state {
        return Ok(Some(node.clone()));
    }
    let node_id = node.id.clone().context("Node missing record ID")?;
    db::update_node_state_from(db, node_id, node.state, to).await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::services::lab_operations::{begin, try_observe};
    use db::{
        DefaultRequest, LabDetails, LabNetworks, create_lab, create_node, create_user,
        upsert_node_image,
    };
    use shared::data::{LabState, NodeConfig, NodeModel, Probe, Refused};

    #[test]
    fn every_libvirt_state_has_one_meaning() {
        let expected = [
            (Some(VIR_DOMAIN_RUNNING), Runtime::Running),
            (Some(VIR_DOMAIN_SHUTOFF), Runtime::Stopped(Halt::Off)),
            (Some(VIR_DOMAIN_PAUSED), Runtime::Stopped(Halt::Paused)),
            (Some(VIR_DOMAIN_CRASHED), Runtime::Faulted("crashed")),
            (
                Some(VIR_DOMAIN_SHUTDOWN),
                Runtime::Transitional("shutting down"),
            ),
            (Some(VIR_DOMAIN_BLOCKED), Runtime::Transitional("blocked")),
            (
                Some(VIR_DOMAIN_PMSUSPENDED),
                Runtime::Transitional("suspended"),
            ),
            (
                Some(VIR_DOMAIN_NOSTATE),
                Runtime::Transitional("in no state"),
            ),
            // M4.2: a domain that does not exist is not a stopped one.
            (None, Runtime::Absent),
        ];
        for (code, runtime) in expected {
            assert_eq!(libvirt_runtime(code), runtime, "{code:?}");
        }
    }

    #[test]
    fn every_container_status_has_one_meaning() {
        let expected = [
            (Some(ContainerStateStatusEnum::RUNNING), Runtime::Running),
            (
                Some(ContainerStateStatusEnum::EXITED),
                Runtime::Stopped(Halt::Off),
            ),
            (
                Some(ContainerStateStatusEnum::CREATED),
                Runtime::Stopped(Halt::Off),
            ),
            (
                Some(ContainerStateStatusEnum::PAUSED),
                Runtime::Stopped(Halt::Paused),
            ),
            // M4.1: a dead container cannot be started, so it is not shown as stopped.
            (
                Some(ContainerStateStatusEnum::DEAD),
                Runtime::Faulted("dead"),
            ),
            (
                Some(ContainerStateStatusEnum::RESTARTING),
                Runtime::Transitional("restarting"),
            ),
            (
                Some(ContainerStateStatusEnum::REMOVING),
                Runtime::Transitional("being removed"),
            ),
            (
                Some(ContainerStateStatusEnum::EMPTY),
                Runtime::Transitional("in no state"),
            ),
            (None, Runtime::Absent),
        ];
        for (status, runtime) in expected {
            assert_eq!(container_runtime(status), runtime, "{status:?}");
        }
    }

    /// A database with one lab of `count` nodes, each `Building` as `up` inserts them.
    pub(crate) async fn lab_of(
        lab_id: &str,
        count: u16,
    ) -> (db::Database, RecordId, Vec<RecordId>) {
        let db = db::open_in_memory().await.unwrap();
        db::apply_schema(&db).await.unwrap();
        let user = create_user(&db, "alice".to_string(), "TestPass123!", false, vec![])
            .await
            .unwrap();
        let networks = LabNetworks {
            loopback_network: "127.127.1.0/24".to_string(),
            management_network: "172.31.1.0/24".to_string(),
            gateway_ipv4: "172.31.1.1".to_string(),
            router_ipv4: "172.31.1.2".to_string(),
            management_network_v6: None,
            loopback_network_v6: None,
            gateway_ipv6: None,
            router_ipv6: None,
        };
        let lab = create_lab(
            &db,
            lab_id,
            lab_id,
            &user,
            &networks,
            &LabDetails::default(),
        )
        .await
        .unwrap();
        let image = upsert_node_image(
            &db,
            NodeConfig::get_model(NodeModel::UbuntuLinux),
            DefaultRequest::Keep,
        )
        .await
        .unwrap();
        let mut nodes = Vec::new();
        for index in 1..=count {
            let node = create_node(
                &db,
                &format!("node{index}"),
                index,
                image.id.clone().unwrap(),
                lab.id.clone().unwrap(),
            )
            .await
            .unwrap();
            nodes.push(node.id.unwrap());
        }
        (db, lab.id.unwrap(), nodes)
    }

    /// The lab's status, derived from its nodes as every reader derives it.
    async fn lab_status(db: &db::Database, lab_id: &str) -> LabState {
        let lab = db::get_lab(db, lab_id).await.unwrap();
        let states: Vec<NodeState> = db::list_nodes_by_lab(db, lab.id.unwrap())
            .await
            .unwrap()
            .iter()
            .map(|node| node.state)
            .collect();
        LabState::derive(&states)
    }

    #[tokio::test]
    async fn an_operation_moves_a_node_and_its_lab_status() {
        // M1.7: the lab status no longer waits for the scanner.
        let (db, _, nodes) = lab_of("lc-ops01", 1).await;
        let operation = begin("lc-ops01").await;
        let node = &nodes[0];

        record(&db, &operation, node, OperationEvent::Launched)
            .await
            .unwrap();
        assert_eq!(lab_status(&db, "lc-ops01").await, LabState::Starting);
        let state = record(&db, &operation, node, OperationEvent::Ready)
            .await
            .unwrap();
        assert_eq!(state, NodeState::Running);
        assert_eq!(lab_status(&db, "lc-ops01").await, LabState::Running);
    }

    #[tokio::test]
    async fn an_operation_cannot_mark_ready_a_node_stopped_meanwhile() {
        // M1.5: `up` probed the node ready, and `down` stopped it before `up` wrote it.
        let (db, _, nodes) = lab_of("lc-ops02", 1).await;
        let up = begin("lc-ops02").await;
        let down = begin("lc-ops02").await;
        let node = &nodes[0];
        record(&db, &up, node, OperationEvent::Launched)
            .await
            .unwrap();
        record(&db, &down, node, OperationEvent::Stopped)
            .await
            .unwrap();

        let error = record(&db, &up, node, OperationEvent::Ready)
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<Refused>(),
            Some(&Refused {
                state: NodeState::Stopped,
                event: OperationEvent::Ready
            })
        );
        assert_eq!(
            db::get_node(&db, node.clone()).await.unwrap().state,
            NodeState::Stopped
        );
    }

    #[tokio::test]
    async fn a_stale_observation_does_not_overwrite_a_newer_write() {
        // M1.6: the scanner read the node, then an operation moved it before the write.
        let (db, _, nodes) = lab_of("lc-obs01", 1).await;
        let stale = db::get_node(&db, nodes[0].clone()).await.unwrap();
        {
            let operation = begin("lc-obs01").await;
            record(&db, &operation, &nodes[0], OperationEvent::Launched)
                .await
                .unwrap();
            record(&db, &operation, &nodes[0], OperationEvent::Stopped)
                .await
                .unwrap();
        }
        let observed = try_observe("lc-obs01").unwrap();
        let running = Observation {
            runtime: Runtime::Running,
            probe: Probe::NotProbed,
        };
        // The scanner read the node `Building`, which it never moves; read it `Starting`.
        let mut stale_starting = stale.clone();
        stale_starting.state = NodeState::Starting;

        let written = observe(&db, &observed, &stale_starting, running)
            .await
            .unwrap();
        assert_eq!(written, None);
        assert_eq!(
            db::get_node(&db, nodes[0].clone()).await.unwrap().state,
            NodeState::Stopped
        );
    }

    #[tokio::test]
    async fn a_lab_is_starting_while_up_builds_it() {
        // M1.1: nothing the scanner sees moves a node `up` is building.
        let (db, _, nodes) = lab_of("lc-obs02", 2).await;
        let observed = try_observe("lc-obs02").unwrap();
        let gone = Observation {
            runtime: Runtime::Absent,
            probe: Probe::NotProbed,
        };
        for node in &nodes {
            let node = db::get_node(&db, node.clone()).await.unwrap();
            assert_eq!(
                observe(&db, &observed, &node, gone).await.unwrap(),
                Some(NodeState::Building)
            );
        }
        assert_eq!(lab_status(&db, "lc-obs02").await, LabState::Starting);
    }

    #[tokio::test]
    async fn a_failed_node_stays_failed_across_scans() {
        // M1.2: `Failed` used to last one scan.
        let (db, _, nodes) = lab_of("lc-obs03", 1).await;
        {
            let operation = begin("lc-obs03").await;
            record(&db, &operation, &nodes[0], OperationEvent::Failed)
                .await
                .unwrap();
        }
        let observed = try_observe("lc-obs03").unwrap();
        for runtime in [
            Runtime::Absent,
            Runtime::Running,
            Runtime::Stopped(Halt::Off),
        ] {
            let node = db::get_node(&db, nodes[0].clone()).await.unwrap();
            let observation = Observation {
                runtime,
                probe: Probe::Ready,
            };
            assert_eq!(
                observe(&db, &observed, &node, observation).await.unwrap(),
                Some(NodeState::Failed)
            );
        }
        assert_eq!(lab_status(&db, "lc-obs03").await, LabState::Failed);
    }
}
