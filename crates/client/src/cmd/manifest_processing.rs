use anyhow::{Result, anyhow};

use shared::data;
use shared::util;
use topology;

/// Process manifest nodes into expanded format with indices assigned
pub fn process_manifest_nodes(manifest_nodes: &[topology::Node]) -> Vec<topology::NodeExpanded> {
    let nodes_expanded = manifest_nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| topology::NodeExpanded {
            name: node.name.clone(),
            model: node.model.clone(),
            // Node indexes start from 1. This aligns with IP address assignment
            index: idx as u16 + 1,
            version: node.version.clone(),
            memory: node.memory,
            cpu_count: node.cpu_count,
            ipv4_address: node.ipv4_address,
            ssh_authorized_keys: node.ssh_authorized_keys.clone(),
            ssh_authorized_key_files: node.ssh_authorized_key_files.clone(),
            text_files: node.text_files.clone(),
            binary_files: node.binary_files.clone(),
            systemd_units: node.systemd_units.clone(),
            image: node.image.clone(),
            privileged: node.privileged,
            environment_variables: node.environment_variables.clone(),
            volumes: node.volumes.clone(),
            commands: node.commands.clone(),
        })
        .collect();
    nodes_expanded
}

/// Process manifest links into detailed link format with resolved interface indices
pub fn process_manifest_links(
    manifest_links: &Option<Vec<topology::Link2>>,
    manifest_nodes: &[topology::NodeExpanded],
) -> Result<Vec<topology::LinkDetailed>> {
    let manifest_links = manifest_links.clone().unwrap_or_default();
    // links from manifest links
    let links = manifest_links
        .iter()
        .map(|x: &topology::Link2| x.expand())
        .collect::<Result<Vec<topology::LinkExpanded>>>()?;

    let mut links_detailed = vec![];
    for (link_idx, link) in links.iter().enumerate() {
        let mut this_link = topology::LinkDetailed::default();
        for device in manifest_nodes.iter() {
            let device_model = device.model.clone();
            // let device_index = manifest_nodes.iter().map()
            if link.node_a == device.name {
                let int_idx = util::interface_to_idx(&device_model, &link.int_a)?;
                let peer_node = manifest_nodes
                    .iter()
                    .find(|n| n.name == link.node_b)
                    .ok_or_else(|| anyhow!("Peer node not found: {}", link.node_b))?;
                this_link.node_a = device.name.clone();
                this_link.node_a_idx = device.index;
                this_link.node_a_model = device_model;
                this_link.int_a = link.int_a.clone();
                this_link.int_a_idx = int_idx;
                this_link.link_idx = link_idx as u16;
                this_link.node_b_idx = peer_node.index;
            } else if link.node_b == device.name {
                let peer_node = manifest_nodes
                    .iter()
                    .find(|n| n.name == link.node_a)
                    .ok_or_else(|| anyhow!("Peer node not found: {}", link.node_a))?;
                let int_idx = util::interface_to_idx(&device_model, &link.int_b)?;
                this_link.node_b = device.name.clone();
                this_link.node_b_idx = device.index;
                this_link.node_b_model = device_model;
                this_link.int_b = link.int_b.clone();
                this_link.int_b_idx = int_idx;
                this_link.link_idx = link_idx as u16;
                this_link.node_a_idx = peer_node.index;
            }
        }
        links_detailed.push(this_link)
    }
    Ok(links_detailed)
}

/// Get node configuration from a list of node configs
pub fn get_node_config(
    node_model: &data::NodeModel,
    data: &[data::NodeConfig],
) -> Result<data::NodeConfig> {
    Ok(data
        .iter()
        .find(|x| &x.model == node_model)
        .ok_or_else(|| anyhow!("Node config not found for model: {}", node_model))?
        .clone())
}
