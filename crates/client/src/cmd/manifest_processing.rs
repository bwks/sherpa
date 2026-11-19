use anyhow::Result;
use tracing::instrument;

/// Process manifest nodes into expanded format with indices assigned, each with its kind.
///
/// # Errors
///
/// Fails when a node sets a `kind` its model does not take.
#[instrument(level = "debug", skip(manifest_nodes))]
pub fn process_manifest_nodes(
    manifest_nodes: &[topology::Node],
) -> Result<Vec<topology::NodeExpanded>> {
    // Node indexes start from 1, which aligns with IP address assignment.
    // check_device_count caps a lab far below u16::MAX nodes.
    (1..=u16::MAX)
        .zip(manifest_nodes)
        .map(|(index, node)| topology::NodeExpanded::new(index, node))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeModel;

    #[test]
    fn test_process_manifest_nodes_assigns_indices_starting_at_one() {
        let nodes = vec![
            topology::Node {
                name: "dev01".to_string(),
                model: NodeModel::UbuntuLinux,
                ..Default::default()
            },
            topology::Node {
                name: "dev02".to_string(),
                model: NodeModel::UbuntuLinux,
                ..Default::default()
            },
            topology::Node {
                name: "dev03".to_string(),
                model: NodeModel::UbuntuLinux,
                ..Default::default()
            },
        ];

        let expanded = process_manifest_nodes(&nodes).unwrap();

        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].index, 1);
        assert_eq!(expanded[0].name, "dev01");
        assert_eq!(expanded[1].index, 2);
        assert_eq!(expanded[1].name, "dev02");
        assert_eq!(expanded[2].index, 3);
        assert_eq!(expanded[2].name, "dev03");
    }
}
