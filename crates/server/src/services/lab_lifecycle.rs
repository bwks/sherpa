//! Moving labs through their lifecycle, and recovering labs and nodes an operation was moving
//! when the server stopped.
//!
//! The allowed moves are in `shared::data::LabPhase::on_event`. Here they are written:
//! `record` applies an operation's event while it holds the lab's lease (`LabHold`), with a
//! compare-and-set on the phase it read. `up` reserves a lab by inserting it `Creating`, and
//! moves it to `Ready` once built; destroy, clean and a failed `up`'s rollback move it through
//! `Destroying`, and remove its record or leave it `DestroyFailed`.
//!
//! Operations on a lab's nodes (`down`, `resume`, redeploy) run only on a `Ready` lab:
//! `require_ready`.
//!
//! At startup no operation runs, so a lab left `Creating` or `Destroying`, or a node left
//! `Building`, has lost its owner. `recover` marks them before the server takes requests.

use anyhow::{Context, Result, bail};
use shared::data::{DbLab, LabEvent, LabPhase, LabTransition, NodeState, OperationEvent, RecordId};
use tracing::instrument;

use crate::services::error::ServiceError;
use crate::services::lab_operations::{self, LabHold};
use crate::services::node_lifecycle;

/// How often a write that lost a race to another writer is tried again.
const WRITE_ATTEMPTS: usize = 3;

/// What `recover` found and marked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Labs `up` was building, now `Interrupted`.
    pub interrupted_labs: usize,
    /// Labs a destroy was removing, now `DestroyFailed`.
    pub failed_destroys: usize,
    /// Nodes `up` or a redeploy was building, now `Failed`.
    pub failed_nodes: usize,
    /// Labs that could not be recovered; each is logged.
    pub errors: usize,
}

/// Apply `event` to the lab `hold` is on, and return what it did.
///
/// # Errors
/// When the lab's phase does not allow the event (its source is the `LabRefused`), when the lab
/// does not exist, or when the database fails.
#[instrument(skip(db, hold), fields(lab_id = %hold.lab_id()), level = "debug")]
pub async fn record(
    db: &db::Database,
    hold: &impl LabHold,
    event: LabEvent,
) -> Result<LabTransition> {
    let lab_id = hold.lab_id();
    for _ in 0..WRITE_ATTEMPTS {
        let lab = db::find_lab(db, lab_id)
            .await?
            .with_context(|| format!("Lab {lab_id} not found"))?;
        let transition = lab.phase.on_event(event)?;
        let written = match transition {
            LabTransition::To(to) if to == lab.phase => true,
            LabTransition::To(to) => db::update_lab_phase_from(db, lab_id, lab.phase, to)
                .await?
                .is_some(),
            LabTransition::Removed => db::delete_lab_from(db, lab_id, lab.phase).await?,
        };
        if written {
            tracing::info!(lab_id = %lab_id, from = %lab.phase, ?transition, "Lab phase changed");
            return Ok(transition);
        }
    }
    bail!("Lab {lab_id} kept changing phase; could not {event} it")
}

/// Refuse an operation on the nodes of `lab` unless `up` has built it and no destroy has
/// started: the refusal says what the user can do.
///
/// # Errors
/// A `ServiceError::Conflict` when the lab is not `Ready`.
pub fn require_ready(lab: &DbLab) -> Result<()> {
    lab.phase
        .ensure_ready()
        .map_err(|not_ready| ServiceError::Conflict(format!("Lab {}: {not_ready}", lab.lab_id)))?;
    Ok(())
}

/// Mark what the operations running when the server stopped left behind: a lab `up` was
/// building is `Interrupted`, a lab a destroy was removing is `DestroyFailed`, and a node `up`
/// or a redeploy was building is `Failed`. Nothing is removed; the user destroys or redeploys.
///
/// Call it at startup, before the server takes requests. It holds each lab's lease while it
/// marks it.
///
/// # Errors
/// When the labs cannot be listed. A lab that cannot be recovered is logged and counted, and
/// the others are still recovered.
#[instrument(skip(db))]
pub async fn recover(db: &db::Database) -> Result<Recovery> {
    let mut recovery = Recovery::default();
    let labs = db::list_labs(db)
        .await
        .context("Failed to list labs to recover")?;
    for lab in &labs {
        if let Err(error) = recover_lab(db, lab, &mut recovery).await {
            recovery.errors += 1;
            tracing::error!(lab_id = %lab.lab_id, error = ?error, "Failed to recover lab");
        }
    }
    Ok(recovery)
}

async fn recover_lab(db: &db::Database, lab: &DbLab, recovery: &mut Recovery) -> Result<()> {
    let operation = lab_operations::begin(&lab.lab_id).await;
    let lab_record = lab.id.clone().context("Lab missing record ID")?;
    match lab.phase {
        LabPhase::Creating => {
            record(db, &operation, LabEvent::OwnerLost).await?;
            recovery.interrupted_labs += 1;
            tracing::warn!(lab_id = %lab.lab_id, "The server stopped while this lab was being created; it is interrupted");
        }
        LabPhase::Destroying => {
            record(db, &operation, LabEvent::OwnerLost).await?;
            recovery.failed_destroys += 1;
            tracing::warn!(lab_id = %lab.lab_id, "The server stopped while this lab was being destroyed; its destroy failed");
        }
        LabPhase::Ready | LabPhase::DestroyFailed | LabPhase::Interrupted | LabPhase::Unknown => {}
    }
    recovery.failed_nodes += fail_building_nodes(db, &operation, lab_record).await?;
    Ok(())
}

/// Record `Failed` for each node of the lab still `Building`, and return how many there were.
async fn fail_building_nodes(
    db: &db::Database,
    operation: &lab_operations::LabOperation,
    lab: RecordId,
) -> Result<usize> {
    let mut failed = 0;
    for node in db::list_nodes_by_lab(db, lab).await? {
        if node.state != NodeState::Building {
            continue;
        }
        let node_id = node.id.context("Node missing record ID")?;
        node_lifecycle::record(db, operation, &node_id, OperationEvent::Failed).await?;
        tracing::warn!(node = %node.name, "The server stopped while this node was being built; it failed");
        failed += 1;
    }
    Ok(failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::lab_operations::begin;
    use crate::services::node_lifecycle::tests::lab_of;
    use shared::data::LabRefused;

    async fn phase(db: &db::Database, lab_id: &str) -> Option<LabPhase> {
        db::find_lab(db, lab_id).await.unwrap().map(|lab| lab.phase)
    }

    async fn node_states(db: &db::Database, lab: &RecordId) -> Vec<NodeState> {
        db::list_nodes_by_lab(db, lab.clone())
            .await
            .unwrap()
            .into_iter()
            .map(|node| node.state)
            .collect()
    }

    /// Move the lab, `Creating` as `up` inserts it, to `to`.
    async fn put_in(db: &db::Database, lab_id: &str, to: LabPhase) {
        if to != LabPhase::Creating {
            db::update_lab_phase_from(db, lab_id, LabPhase::Creating, to)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn up_moves_the_lab_it_reserved_to_ready() {
        let (db, _, _) = lab_of("ll-built", 1).await;
        let operation = begin("ll-built").await;

        let moved = record(&db, &operation, LabEvent::Built).await.unwrap();

        assert_eq!(moved, LabTransition::To(LabPhase::Ready));
        assert_eq!(phase(&db, "ll-built").await, Some(LabPhase::Ready));
    }

    #[tokio::test]
    async fn an_up_whose_lab_is_already_built_cannot_release_it() {
        // M3.1: a second `up` must never give back, or roll back, a lab it did not reserve.
        let (db, _, _) = lab_of("ll-taken", 1).await;
        put_in(&db, "ll-taken", LabPhase::Ready).await;
        let second = begin("ll-taken").await;

        let error = record(&db, &second, LabEvent::Released).await.unwrap_err();

        assert_eq!(
            error.downcast_ref::<LabRefused>(),
            Some(&LabRefused {
                phase: LabPhase::Ready,
                event: LabEvent::Released
            })
        );
        assert_eq!(phase(&db, "ll-taken").await, Some(LabPhase::Ready));
    }

    #[tokio::test]
    async fn a_destroyed_lab_loses_its_record_and_a_failed_one_keeps_it() {
        // M6.2 and M7.3: a destroy that left something behind is visible and can be retried.
        let (db, _, _) = lab_of("ll-dfail", 1).await;
        let (_, destroy) =
            lab_operations::stop("ll-dfail", lab_operations::StopReason::Destroy).await;
        record(&db, &destroy, LabEvent::DestroyStarted)
            .await
            .unwrap();
        record(&db, &destroy, LabEvent::DestroyFailed)
            .await
            .unwrap();
        assert_eq!(phase(&db, "ll-dfail").await, Some(LabPhase::DestroyFailed));

        record(&db, &destroy, LabEvent::DestroyStarted)
            .await
            .unwrap();
        let removed = record(&db, &destroy, LabEvent::Destroyed).await.unwrap();
        assert_eq!(removed, LabTransition::Removed);
        assert_eq!(phase(&db, "ll-dfail").await, None);
    }

    #[tokio::test]
    async fn operations_on_nodes_need_a_ready_lab() {
        // M7.3: a lab whose destroy failed used to offer Start.
        let (db, _, _) = lab_of("ll-ready", 1).await;
        for to in [
            LabPhase::Creating,
            LabPhase::DestroyFailed,
            LabPhase::Interrupted,
            LabPhase::Ready,
        ] {
            let mut lab = db::get_lab(&db, "ll-ready").await.unwrap();
            lab.phase = to;
            let result = require_ready(&lab);
            assert_eq!(result.is_ok(), to == LabPhase::Ready, "{to:?}");
            if let Err(error) = result {
                assert!(
                    matches!(ServiceError::find(&error), Some(ServiceError::Conflict(_))),
                    "{error}"
                );
            }
        }
    }

    #[tokio::test]
    async fn recovery_leaves_no_lab_or_node_without_an_owner() {
        // M6.3: the daemon stopped during `up`, during a destroy, and during a redeploy.
        let (db, creating, _) = lab_of("ll-rec01", 2).await;
        // `lab_of` makes one lab per database; add the others to the same one.
        let user = db::get_user(&db, "alice").await.unwrap();
        let mut labs = vec![("ll-rec01", creating, LabPhase::Creating)];
        for (index, (lab_id, to)) in [
            ("ll-rec02", LabPhase::Destroying),
            ("ll-rec03", LabPhase::Ready),
            ("ll-rec04", LabPhase::DestroyFailed),
        ]
        .into_iter()
        .enumerate()
        {
            let octet = index + 2;
            let lab = db::create_lab(
                &db,
                lab_id,
                lab_id,
                &user,
                &db::LabNetworks {
                    loopback_network: format!("127.127.{octet}.0/24"),
                    management_network: format!("172.31.{octet}.0/24"),
                    gateway_ipv4: format!("172.31.{octet}.1"),
                    router_ipv4: format!("172.31.{octet}.2"),
                    management_network_v6: None,
                    loopback_network_v6: None,
                    gateway_ipv6: None,
                    router_ipv6: None,
                },
                &db::LabDetails::default(),
            )
            .await
            .unwrap();
            put_in(&db, lab_id, to).await;
            labs.push((lab_id, lab.id.unwrap(), to));
        }
        // The ready lab has a running node and one a redeploy was rebuilding.
        let image = db::list_node_images(&db).await.unwrap().remove(0);
        let ready = labs[2].1.clone();
        let running = db::create_node(&db, "r1", 1, image.id.clone().unwrap(), ready.clone())
            .await
            .unwrap();
        db::update_node_state_from(
            &db,
            running.id.unwrap(),
            NodeState::Building,
            NodeState::Running,
        )
        .await
        .unwrap();
        db::create_node(&db, "r2", 2, image.id.unwrap(), ready.clone())
            .await
            .unwrap();

        let recovery = recover(&db).await.unwrap();

        assert_eq!(
            recovery,
            Recovery {
                interrupted_labs: 1,
                failed_destroys: 1,
                failed_nodes: 3,
                errors: 0,
            }
        );
        assert_eq!(phase(&db, "ll-rec01").await, Some(LabPhase::Interrupted));
        assert_eq!(phase(&db, "ll-rec02").await, Some(LabPhase::DestroyFailed));
        assert_eq!(phase(&db, "ll-rec03").await, Some(LabPhase::Ready));
        assert_eq!(phase(&db, "ll-rec04").await, Some(LabPhase::DestroyFailed));
        let mut ready_states = node_states(&db, &ready).await;
        ready_states.sort_by_key(ToString::to_string);
        assert_eq!(ready_states, [NodeState::Failed, NodeState::Running]);
        for (lab_id, lab, _) in &labs {
            let phase = phase(&db, lab_id).await.unwrap();
            assert!(
                !matches!(phase, LabPhase::Creating | LabPhase::Destroying),
                "{lab_id} {phase:?}"
            );
            assert!(
                !node_states(&db, lab).await.contains(&NodeState::Building),
                "{lab_id}"
            );
        }

        // Recovering again finds nothing left to mark.
        assert_eq!(recover(&db).await.unwrap(), Recovery::default());
    }
}
