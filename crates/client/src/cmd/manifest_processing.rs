use anyhow::{Context, Result};
use tracing::instrument;

use shared::data::ModelKind;
use topology::NodeExpanded;
use topology::validate::{
    check_bridges, check_duplicate_device, check_lab_name, check_links_to_itself,
    check_node_file_settings, check_nonzero_node_settings, check_ready_timeout,
    check_text_file_modes, validate_environment_variables, validate_provisioners,
};

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
        .map(|(index, node)| NodeExpanded::new(index, node))
        .collect()
}

/// Check each node's `text_files`, `startup_scripts` and `systemd_units` against its model and
/// kind's built-in image. The client does not know the images on the server, so the server
/// checks again against the image it builds the node from.
///
/// # Errors
///
/// Fails when a node sets a `kind` its model does not take, or sets one of these settings its
/// first-boot config does not take.
pub fn check_file_settings(nodes: &[topology::Node]) -> Result<()> {
    for node in nodes {
        let model_kind = ModelKind::new(node.model, node.kind)
            .with_context(|| format!("Manifest - node: '{}'", node.name))?;
        check_node_file_settings(node, &model_kind.node_config())?;
    }
    Ok(())
}

/// Run the manifest checks `up` and `redeploy` share before any file is read or the server is
/// called: the lab name, duplicate node names, `ready_timeout`, zero node settings, a link to
/// itself, the bridges' names and links, provisioners, text file modes, environment variable
/// keys and each node's file settings.
///
/// # Errors
///
/// Fails on the first check the manifest does not pass, saying which.
pub fn check_manifest(manifest: &topology::Manifest) -> Result<()> {
    check_lab_name(&manifest.name).context("Manifest validation failed: lab name")?;
    check_duplicate_device(&manifest.nodes)
        .context("Manifest validation failed: duplicate nodes")?;
    check_ready_timeout(manifest.ready_timeout)
        .context("Manifest validation failed: ready timeout")?;
    check_nonzero_node_settings(&manifest.nodes)
        .context("Manifest validation failed: node settings")?;
    check_links_to_itself(manifest.links.as_deref().unwrap_or_default())
        .context("Manifest validation failed: links")?;
    check_bridges(manifest.bridges.as_deref().unwrap_or_default())
        .context("Manifest validation failed: bridges")?;
    validate_provisioners(manifest).context("Manifest validation failed: provisioners")?;
    check_text_file_modes(&manifest.nodes).context("Manifest validation failed: text files")?;
    validate_environment_variables(&manifest.nodes)
        .context("Manifest validation failed: environment variables")?;
    check_file_settings(&manifest.nodes).context("Manifest validation failed: node settings")
}

#[cfg(test)]
mod tests {
    use super::*;
    use topology::{Bridge, Manifest, TopologyError};

    fn node(name: &str) -> topology::Node {
        topology::Node {
            name: name.to_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn manifest_links_020_the_cli_refuses_a_bridge_with_one_link_at_up_and_redeploy() {
        let manifest = Manifest {
            name: "lab1".to_owned(),
            nodes: vec![node("r1")],
            bridges: Some(vec![Bridge {
                name: "br1".to_owned(),
                links: vec!["r1::eth1".to_owned()],
            }]),
            ..Default::default()
        };

        let error = check_manifest(&manifest).unwrap_err();

        assert_eq!(
            error.downcast_ref::<TopologyError>(),
            Some(&TopologyError::BridgeTooFewLinks {
                bridge: "br1".to_owned(),
                links: 1,
            }),
            "{error:#}"
        );
    }
}
