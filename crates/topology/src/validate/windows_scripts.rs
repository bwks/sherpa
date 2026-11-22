use anyhow::{Result, bail};

use crate::Node;
use shared::data::NodeModel;

/// Check that no Windows node has a startup script whose file name contains `"` or `%`. The
/// name goes into the `cmd` line cloudbase-init runs the script with, where `"` ends the quoted
/// path and `%` expands a variable, so such a name is refused rather than quoted. The names
/// checked are those of the scripts the client read.
///
/// # Errors
///
/// Fails on the first such script, naming the node and the script.
pub fn check_windows_startup_script_names(nodes: &[Node]) -> Result<()> {
    for node in nodes {
        if !matches!(
            node.model,
            NodeModel::DevboxWindows | NodeModel::WindowsServer
        ) {
            continue;
        }
        for script in node.startup_scripts_data.iter().flatten() {
            if script.filename.contains(['"', '%']) {
                bail!(
                    r#"Manifest - node: '{}' startup script '{}' contains '"' or '%', which cannot be used in a script name on a Windows node"#,
                    node.name,
                    script.filename
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::StartupScript;

    use super::*;

    fn windows_node(filename: &str) -> Node {
        Node {
            name: "win1".to_string(),
            model: NodeModel::WindowsServer,
            startup_scripts_data: Some(vec![StartupScript {
                filename: filename.to_string(),
                content: "V3JpdGUtSG9zdCBoaQ==".to_string(),
            }]),
            ..Node::default()
        }
    }

    #[test]
    fn boot_scripts_006_a_windows_startup_script_name_with_a_quote_or_percent_is_refused() {
        for filename in [r#"a" & calc & ".ps1"#, "%TEMP%.ps1"] {
            let error = check_windows_startup_script_names(&[windows_node(filename)])
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("'win1' startup script '{filename}'")),
                "{error}"
            );
        }
        check_windows_startup_script_names(&[windows_node("setup.ps1")]).unwrap();
    }
}
