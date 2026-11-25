use anyhow::{Context, Result};

use crate::{Node, TopologyError};
use shared::validators::mode_from_octal_digits;

/// Check every node's text file modes are octal digits up to 777, as manifests write modes.
/// Both the files as written and the contents the client read are checked, since the server
/// sees only the latter.
///
/// # Errors
///
/// Fails on the first text file whose `permissions` are not octal digits up to 777.
pub fn check_text_file_modes(nodes: &[Node]) -> Result<()> {
    for node in nodes {
        let files = node
            .text_files
            .iter()
            .flatten()
            .map(|file| (file.dst.as_str(), file.permissions));
        let read = node
            .text_files_data
            .iter()
            .flatten()
            .map(|file| (file.dst.as_str(), file.permissions));
        for (dst, permissions) in files.chain(read) {
            mode_from_octal_digits(permissions).with_context(|| {
                TopologyError::TextFilePermissions {
                    node: node.name.clone(),
                    dst: dst.to_owned(),
                }
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{TextFile, TextFileData};
    use shared::SharedError;

    use super::*;

    fn refused(dst: &str) -> TopologyError {
        TopologyError::TextFilePermissions {
            node: "dev01".to_owned(),
            dst: dst.to_owned(),
        }
    }

    fn node(text_file: Option<u32>, read_file: Option<u32>) -> Node {
        Node {
            name: "dev01".to_string(),
            text_files: text_file.map(|permissions| {
                vec![TextFile {
                    dst: "/etc/motd".to_string(),
                    permissions,
                    ..TextFile::default()
                }]
            }),
            text_files_data: read_file.map(|permissions| {
                vec![TextFileData {
                    dst: "/etc/issue".to_string(),
                    permissions,
                    ..TextFileData::default()
                }]
            }),
            ..Node::default()
        }
    }

    #[test]
    fn boot_settings_001_a_text_file_whose_permissions_are_not_octal_digits_is_refused() {
        for (node, dst, digits) in [
            (node(Some(999), None), "/etc/motd", 999),
            (node(None, Some(1000)), "/etc/issue", 1000),
        ] {
            let error = check_text_file_modes(&[node]).unwrap_err();
            assert_eq!(error.downcast_ref::<TopologyError>(), Some(&refused(dst)));
            assert_eq!(
                error.root_cause().downcast_ref::<SharedError>(),
                Some(&SharedError::ModeNotOctal { digits })
            );
        }
    }
}
