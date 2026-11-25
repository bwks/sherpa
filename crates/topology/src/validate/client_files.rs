use anyhow::{Result, bail};

use crate::{Node, TopologyError};

/// Check the client read every file a node lists, and named the files it sent plainly.
///
/// Manifest paths are paths on the client. The server never opens them: the Sherpa CLI reads
/// each file and sends its contents in the matching `*_data` field. A manifest that lists files
/// without their contents did not come through the CLI, for example one pasted into the web UI.
///
/// # Errors
///
/// Fails when a node lists files the client did not send, or sends a systemd unit or startup
/// script whose name is not a plain file name.
pub fn check_client_read_files(nodes: &[Node]) -> Result<()> {
    for node in nodes {
        let lists = [
            (
                "text_files",
                count(node.text_files.as_ref()),
                count(node.text_files_data.as_ref()),
            ),
            (
                "systemd_units",
                count(node.systemd_units.as_ref()),
                count(node.systemd_units_data.as_ref()),
            ),
            (
                "startup_scripts",
                count(node.startup_scripts.as_ref()),
                count(node.startup_scripts_data.as_ref()),
            ),
        ];
        for (field, listed, read) in lists {
            if listed != read {
                bail!(TopologyError::FilesNotRead {
                    node: node.name.clone(),
                    field,
                    listed,
                    read,
                });
            }
        }
        for unit in node.systemd_units_data.iter().flatten() {
            check_file_name(&node.name, &unit.name)?;
        }
        for script in node.startup_scripts_data.iter().flatten() {
            check_file_name(&node.name, &script.filename)?;
        }
    }
    Ok(())
}

fn count<T>(files: Option<&Vec<T>>) -> usize {
    files.map_or(0, Vec::len)
}

/// A name the server writes a file under: no path separators, no `.` or `..`, and no leading
/// `-`, which tools such as `e2cp` would read as an option.
fn check_file_name(node: &str, name: &str) -> Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('-')
        || name.contains(['/', '\\', '\0'])
    {
        bail!("Manifest - node: '{node}' file name '{name}' is not a plain file name");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StartupScript, SystemdUnit, SystemdUnitData, TextFile};

    fn node() -> Node {
        Node {
            name: "fc1".to_owned(),
            ..Default::default()
        }
    }

    fn unit(name: &str) -> SystemdUnitData {
        SystemdUnitData {
            name: name.to_owned(),
            content: String::new(),
            enabled: true,
        }
    }

    #[test]
    fn client_content_011_files_listed_without_their_contents_are_refused() {
        let listed = [
            (
                "text_files",
                Node {
                    text_files: Some(vec![TextFile::default()]),
                    ..node()
                },
            ),
            (
                "systemd_units",
                Node {
                    systemd_units: Some(vec![SystemdUnit::default()]),
                    ..node()
                },
            ),
            (
                "startup_scripts",
                Node {
                    startup_scripts: Some(vec!["boot.sh".to_owned()]),
                    ..node()
                },
            ),
        ];
        for (field, node) in listed {
            let error = check_client_read_files(&[node]).expect_err("unread files accepted");
            assert_eq!(
                error.downcast_ref::<TopologyError>(),
                Some(&TopologyError::FilesNotRead {
                    node: "fc1".to_owned(),
                    field,
                    listed: 1,
                    read: 0,
                })
            );
        }
    }

    #[test]
    fn client_content_012_names_that_are_paths_or_options_are_refused() {
        for name in ["", ".", "..", "../x", "a/b", "-o", "a\\b"] {
            let node = Node {
                systemd_units: Some(vec![SystemdUnit::default()]),
                systemd_units_data: Some(vec![unit(name)]),
                ..node()
            };
            assert!(
                check_client_read_files(&[node]).is_err(),
                "'{name}' accepted"
            );
        }
    }

    #[test]
    fn client_content_014_startup_scripts_named_as_paths_or_options_are_refused() {
        for name in ["", ".", "..", "../x", "a/b", "-o", "a\\b"] {
            let node = Node {
                startup_scripts: Some(vec!["boot.sh".to_owned()]),
                startup_scripts_data: Some(vec![StartupScript {
                    filename: name.to_owned(),
                    content: String::new(),
                }]),
                ..node()
            };
            assert!(
                check_client_read_files(&[node]).is_err(),
                "'{name}' accepted"
            );
        }
    }
}
