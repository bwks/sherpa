use anyhow::{Result, bail};

use crate::Node;

/// Check the client read every file a node lists, and named the files it sent plainly.
///
/// Manifest paths are paths on the client. The server never opens them: the Sherpa CLI reads
/// each file and sends its contents in the matching `*_data` field. A manifest that lists files
/// without their contents did not come through the CLI, for example one pasted into the web UI.
///
/// # Errors
///
/// Fails when a node lists files the client did not send, or sends a binary file or systemd
/// unit whose name is not a plain file name.
pub fn check_client_read_files(nodes: &[Node]) -> Result<()> {
    for node in nodes {
        let lists = [
            (
                "text_files",
                count(node.text_files.as_ref()),
                count(node.text_files_data.as_ref()),
            ),
            (
                "binary_files",
                count(node.binary_files.as_ref()),
                count(node.binary_files_data.as_ref()),
            ),
            (
                "systemd_units",
                count(node.systemd_units.as_ref()),
                count(node.systemd_units_data.as_ref()),
            ),
            (
                "ssh_authorized_key_files",
                count(node.ssh_authorized_key_files.as_ref()),
                count(node.ssh_authorized_key_files_data.as_ref()),
            ),
            (
                "startup_scripts",
                count(node.startup_scripts.as_ref()),
                count(node.startup_scripts_data.as_ref()),
            ),
            (
                "user_scripts",
                count(node.user_scripts.as_ref()),
                count(node.user_scripts_data.as_ref()),
            ),
        ];
        for (field, listed, read) in lists {
            if listed != read {
                bail!(
                    "Manifest - node: '{}' lists {listed} {field} but {read} were read; the Sherpa \
                     CLI reads these files on the client, so start the lab with `sherpa up`",
                    node.name
                );
            }
        }
        let binary_names = node
            .binary_files_data
            .iter()
            .flatten()
            .map(|file| &file.filename);
        let unit_names = node
            .systemd_units_data
            .iter()
            .flatten()
            .map(|unit| &unit.name);
        for name in binary_names.chain(unit_names) {
            check_file_name(&node.name, name)?;
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
    use crate::{AuthorizedKeyFile, BinaryFile, BinaryFileData, SystemdUnit, SystemdUnitData};

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
    fn files_the_client_read_pass() {
        let node = Node {
            binary_files: Some(vec![BinaryFile {
                source: "bin/tool".to_owned(),
            }]),
            binary_files_data: Some(vec![BinaryFileData {
                filename: "tool".to_owned(),
                content: String::new(),
            }]),
            systemd_units: Some(vec![SystemdUnit {
                name: "app.service".to_owned(),
                source: "units/app.service".to_owned(),
                enabled: true,
            }]),
            systemd_units_data: Some(vec![unit("app.service")]),
            ..node()
        };
        assert!(check_client_read_files(&[node, self::node()]).is_ok());
    }

    #[test]
    fn files_listed_without_their_contents_are_refused() {
        let node = Node {
            ssh_authorized_key_files: Some(vec![AuthorizedKeyFile {
                source: "/etc/sherpa/ssh/sherpa_ssh_key.pub".to_owned(),
            }]),
            ..node()
        };
        let error = check_client_read_files(&[node]).expect_err("unread key file accepted");
        assert!(
            error.to_string().contains("ssh_authorized_key_files"),
            "{error}"
        );
    }

    #[test]
    fn names_that_are_paths_or_options_are_refused() {
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
}
