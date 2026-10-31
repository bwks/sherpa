//! Turn server results that report a failure into a command error, so the CLI exits non-zero.
//!
//! Commands print the server's summary first, then call these, so the user sees what failed.

use anyhow::{Result, bail};

use shared::data::{DestroyResponse, NodeActionResult};

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

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{DestroyError, DestroySummary};

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
