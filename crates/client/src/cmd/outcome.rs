//! Turn server results that report a failure into a command error, so the CLI exits non-zero,
//! and print the parts of a result that do not fail the command, such as links left unattached.
//!
//! Commands print the server's summary first, then call these, so the user sees what failed.

use anyhow::{Result, bail, ensure};

use shared::data::{DestroyResponse, LinkReport, LinkState, NodeActionResult, ScannedImage};
use shared::util::Emoji;

use crate::error::ClientError;

/// Fail unless the server destroyed every resource of the lab. `retry` is the command that
/// tries again, such as `sherpa destroy`.
pub(crate) fn ensure_lab_destroyed(response: &DestroyResponse, retry: &str) -> Result<()> {
    if response.success {
        return Ok(());
    }
    let lab_name = response.lab_name.clone();
    let lab_id = response.lab_id.clone();
    if response.summary.database_records_deleted {
        bail!(ClientError::LabResourcesLeft { lab_name, lab_id });
    }
    bail!(ClientError::LabNotDestroyed {
        lab_name,
        lab_id,
        retry: retry.to_owned(),
    });
}

/// Fail if any node's action failed. `action` names it for the message, such as `stopped`.
pub(crate) fn ensure_nodes_succeeded(action: &str, results: &[NodeActionResult]) -> Result<()> {
    let failed: Vec<String> = results
        .iter()
        .filter(|result| !result.success)
        .map(|result| result.name.clone())
        .collect();
    ensure!(
        failed.is_empty(),
        ClientError::NodesFailed {
            action: action.to_owned(),
            failed,
            total: results.len(),
        }
    );
    Ok(())
}

/// Fail if an image scan failed to record any version it found, naming each.
pub(crate) fn ensure_images_recorded(scanned: &[ScannedImage]) -> Result<()> {
    let unrecorded: Vec<ScannedImage> = scanned
        .iter()
        .filter(|image| image.failed)
        .cloned()
        .collect();
    ensure!(
        unrecorded.is_empty(),
        ClientError::ImagesNotRecorded {
            unrecorded,
            total: scanned.len(),
        }
    );
    Ok(())
}

/// The line that tells the user about a link that is not attached, or `None` for one that is.
fn link_note(report: &LinkReport) -> Option<String> {
    match report.state {
        LinkState::Attached => None,
        LinkState::PeerDown { .. } => Some(format!("   {} {report}", Emoji::Info)),
        LinkState::Failed { .. } => Some(format!("   {} {report}", Emoji::Warning)),
    }
}

/// Print each link that is not attached, under the node's line. A link that waits for its peer
/// is expected; a failed one carries no traffic until either end is resumed or redeployed. Not
/// attached links do not fail the command: the node is up.
pub(crate) fn print_link_reports(links: &[LinkReport]) {
    for report in links {
        if let Some(note) = link_note(report) {
            if report.failed() {
                eprintln!("{note}");
            } else {
                println!("{note}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{DestroyError, DestroySummary, NodeKind, NodeModel};

    use crate::cmd::exit_status;

    #[test]
    fn link_state_008_links_that_are_not_attached_are_told_as_information_or_warning() {
        let report = |state| LinkReport {
            link: "vm1::eth1 <-> vm2::eth1".to_string(),
            state,
        };
        assert_eq!(link_note(&report(LinkState::Attached)), None);
        let peer_down = link_note(&report(LinkState::PeerDown {
            peer: "vm2".to_string(),
        }))
        .unwrap();
        assert!(peer_down.contains("vm2 is down"), "{peer_down}");
        let failed = link_note(&report(LinkState::Failed {
            reason: "netlink refused".to_string(),
        }))
        .unwrap();
        assert!(failed.contains("not attached: netlink refused"), "{failed}");

        let report = |state| LinkReport {
            link: "vm1::eth1 <-> vm2::eth1".to_string(),
            state,
        };
        assert_eq!(
            report(LinkState::PeerDown {
                peer: "vm2".to_string()
            })
            .to_string(),
            "link vm1::eth1 <-> vm2::eth1 not attached: vm2 is down; it is attached when vm2 starts"
        );
        let failed = report(LinkState::Failed {
            reason: "no tap".to_string(),
        });
        assert!(failed.failed());
        assert_eq!(
            failed.to_string(),
            "link vm1::eth1 <-> vm2::eth1 not attached: no tap"
        );
        assert!(!report(LinkState::Attached).failed());
    }

    fn destroy_response(success: bool, database_records_deleted: bool) -> DestroyResponse {
        DestroyResponse {
            success,
            lab_id: "e9f870b2".to_string(),
            lab_name: "bridges".to_string(),
            summary: DestroySummary {
                database_records_deleted,
                ..Default::default()
            },
            errors: if success {
                vec![]
            } else {
                vec![DestroyError::new(
                    "database",
                    "e9f870b2",
                    "failed".to_string(),
                )]
            },
        }
    }

    fn node_result(name: &str, success: bool) -> NodeActionResult {
        NodeActionResult {
            name: name.to_string(),
            success,
            message: String::new(),
            links: Vec::new(),
        }
    }

    #[test]
    fn test_lab_left_on_the_server_is_an_error_naming_the_retry() {
        let error =
            ensure_lab_destroyed(&destroy_response(false, false), "sherpa destroy").unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::LabNotDestroyed { lab_name, lab_id, retry })
                    if lab_name == "bridges" && lab_id == "e9f870b2" && retry == "sherpa destroy"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn test_removed_lab_with_leftover_resources_is_an_error() {
        let error =
            ensure_lab_destroyed(&destroy_response(false, true), "sherpa destroy").unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ClientError>(),
                Some(ClientError::LabResourcesLeft { lab_name, lab_id })
                    if lab_name == "bridges" && lab_id == "e9f870b2"
            ),
            "{error:#}"
        );
    }

    #[test]
    fn test_a_failed_node_is_an_error_naming_it() {
        let results = [
            node_result("dev01", true),
            node_result("dev02", false),
            node_result("dev03", false),
        ];
        let error = ensure_nodes_succeeded("stopped", &results).unwrap_err();
        let Some(ClientError::NodesFailed {
            action,
            failed,
            total,
        }) = error.downcast_ref::<ClientError>()
        else {
            panic!("not failed nodes: {error:#}");
        };
        assert_eq!(action, "stopped");
        assert_eq!(failed, &["dev02", "dev03"]);
        assert_eq!(*total, 3);
    }

    fn scanned(version: &str, status: &str, failed: bool) -> ScannedImage {
        ScannedImage {
            model: NodeModel::NatsBus,
            version: version.to_string(),
            kind: NodeKind::Container,
            status: status.to_string(),
            failed,
        }
    }

    #[test]
    fn image_scan_020_a_scan_with_a_version_it_failed_to_record_exits_non_zero_carrying_it() {
        let scanned = [
            scanned("2.10", "imported (default)", false),
            scanned("2.11", "error: database is locked", true),
        ];

        let error = ensure_images_recorded(&scanned).unwrap_err();

        assert_eq!(exit_status(&error), 1);
        let Some(ClientError::ImagesNotRecorded { unrecorded, total }) =
            error.downcast_ref::<ClientError>()
        else {
            panic!("not unrecorded images: {error:#}");
        };
        let unrecorded: Vec<_> = unrecorded
            .iter()
            .map(|image| (image.model, image.kind, image.version.as_str()))
            .collect();
        assert_eq!(
            unrecorded,
            [(NodeModel::NatsBus, NodeKind::Container, "2.11")]
        );
        assert_eq!(*total, 2);
    }

    #[test]
    fn image_scan_021_a_scan_that_recorded_or_found_every_version_exits_0() {
        let scanned = [
            scanned("2.10", "already_exists", false),
            scanned("2.11", "imported", false),
            scanned("2.12", "would_import", false),
        ];

        assert!(ensure_images_recorded(&scanned).is_ok());
        assert!(ensure_images_recorded(&[]).is_ok());
    }
}
