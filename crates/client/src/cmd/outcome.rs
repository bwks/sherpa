//! Turn server results that report a failure into a command error, so the CLI exits non-zero,
//! and print the parts of a result that do not fail the command, such as links left unattached.
//!
//! Commands print the server's summary first, then call these, so the user sees what failed.

use anyhow::{Result, bail};

use shared::data::{DestroyResponse, LinkReport, LinkState, NodeActionResult};
use shared::util::Emoji;

/// Fail unless the server destroyed every resource of the lab. `retry` is the command that
/// tries again, such as `sherpa destroy`.
pub(crate) fn ensure_lab_destroyed(response: &DestroyResponse, retry: &str) -> Result<()> {
    if response.success {
        return Ok(());
    }
    let lab = format!("{}-{}", response.lab_name, response.lab_id);
    if response.summary.database_records_deleted {
        bail!(
            "Lab {lab} was removed, but not all of its resources were cleaned up; see the errors \
             above"
        );
    }
    bail!(
        "Lab {lab} was not fully destroyed and still exists on the server; fix the problem, \
         then run `{retry}` again"
    );
}

/// Fail if any node's action failed. `action` names it for the message, such as `stopped`.
pub(crate) fn ensure_nodes_succeeded(action: &str, results: &[NodeActionResult]) -> Result<()> {
    let mut failed = vec![];
    for result in results {
        if !result.success {
            failed.push(result.name.as_str());
        }
    }
    if !failed.is_empty() {
        bail!(
            "{} of {} nodes were not {action}: {}",
            failed.len(),
            results.len(),
            failed.join(", ")
        );
    }
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
    use shared::data::{DestroyError, DestroySummary};

    #[test]
    fn only_links_that_are_not_attached_are_told() {
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
    fn test_destroyed_lab_is_ok() {
        assert!(ensure_lab_destroyed(&destroy_response(true, true), "sherpa destroy").is_ok());
    }

    #[test]
    fn test_lab_left_on_the_server_is_an_error_naming_the_retry() {
        let error = ensure_lab_destroyed(&destroy_response(false, false), "sherpa destroy")
            .unwrap_err()
            .to_string();
        assert!(error.contains("bridges-e9f870b2"), "{error}");
        assert!(error.contains("still exists"), "{error}");
        assert!(error.contains("`sherpa destroy`"), "{error}");
    }

    #[test]
    fn test_removed_lab_with_leftover_resources_is_an_error() {
        let error = ensure_lab_destroyed(&destroy_response(false, true), "sherpa destroy")
            .unwrap_err()
            .to_string();
        assert!(error.contains("bridges-e9f870b2"), "{error}");
        assert!(error.contains("not all of its resources"), "{error}");
    }

    #[test]
    fn test_all_nodes_succeeding_is_ok() {
        let results = [node_result("dev01", true), node_result("dev02", true)];
        assert!(ensure_nodes_succeeded("stopped", &results).is_ok());
    }

    #[test]
    fn test_a_failed_node_is_an_error_naming_it() {
        let results = [
            node_result("dev01", true),
            node_result("dev02", false),
            node_result("dev03", false),
        ];
        let error = ensure_nodes_succeeded("stopped", &results)
            .unwrap_err()
            .to_string();
        assert!(error.contains("2 of 3 nodes were not stopped"), "{error}");
        assert!(error.contains("dev02, dev03"), "{error}");
    }
}
