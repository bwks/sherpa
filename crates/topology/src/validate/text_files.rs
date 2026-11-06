use anyhow::{Result, ensure};

use crate::Node;
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
            ensure!(
                mode_from_octal_digits(permissions).is_some(),
                "Manifest - node: '{}' text file '{dst}' has permissions {permissions}; use octal \
                 digits up to 777, such as 644",
                node.name
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{TextFile, TextFileData};

    use super::*;

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
    fn octal_text_file_modes_are_accepted() {
        check_text_file_modes(&[node(Some(644), Some(600)), node(None, None)]).unwrap();
    }

    #[test]
    fn text_file_modes_that_are_not_octal_digits_are_refused() {
        let error = check_text_file_modes(&[node(Some(999), None)])
            .unwrap_err()
            .to_string();
        assert!(error.contains("/etc/motd"), "{error}");
        let error = check_text_file_modes(&[node(None, Some(1000))])
            .unwrap_err()
            .to_string();
        assert!(error.contains("/etc/issue"), "{error}");
    }
}
