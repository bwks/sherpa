use anyhow::{Result, anyhow};

use shared::data;
use shared::konst::BRIDGE_PREFIX;
use shared::util;

/// Process manifest nodes into expanded format with indices assigned
pub fn process_manifest_nodes(manifest_nodes: &[topology::Node]) -> Vec<topology::NodeExpanded> {
    manifest_nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| topology::NodeExpanded {
            name: node.name.clone(),
            model: node.model,
            // Node indexes start from 1. This aligns with IP address assignment
            index: idx as u16 + 1,
            version: node.version.clone(),
            memory: node.memory,
            cpu_count: node.cpu_count,
            boot_disk_size: node.boot_disk_size,
            ipv4_address: node.ipv4_address,
            ipv6_address: node.ipv6_address,
            ssh_authorized_keys: node.ssh_authorized_keys.clone(),
            ssh_authorized_key_files: node.ssh_authorized_key_files.clone(),
            text_files: node.text_files_data.clone(),
            binary_files: node.binary_files.clone(),
            systemd_units: node.systemd_units.clone(),
            image: node.image.clone(),
            privileged: node.privileged,
            shm_size: node.shm_size,
            environment_variables: node.environment_variables.clone(),
            volumes: node.volumes.clone(),
            commands: node.commands.clone(),
            user: node.user.clone(),
            skip_ready_check: node.skip_ready_check,
            ztp_config: node.ztp_config.clone(),
            startup_scripts: node.startup_scripts_data.clone(),
            user_scripts: node.user_scripts_data.clone(),
            kernel_cmdline: node.kernel_cmdline.clone(),
            ready_port: node.ready_port,
        })
        .collect()
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
            let device_model = device.model;
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

/// Get node image from a list of node images
pub fn get_node_image(
    node_model: &data::NodeModel,
    data: &[data::NodeConfig],
) -> Result<data::NodeConfig> {
    Ok(data
        .iter()
        .find(|x| &x.model == node_model && x.default)
        .ok_or_else(|| anyhow!("Default node image not found for model: {}", node_model))?
        .clone())
}

/// Process manifest bridges into detailed bridge format with resolved interface indices
pub fn process_manifest_bridges(
    manifest_bridges: &Option<Vec<topology::Bridge>>,
    manifest_nodes: &[topology::NodeExpanded],
    lab_id: &str,
) -> Result<Vec<topology::BridgeDetailed>> {
    let manifest_bridges = manifest_bridges.clone().unwrap_or_default();
    let bridges = manifest_bridges
        .iter()
        .map(|x: &topology::Bridge| x.parse_links())
        .collect::<Result<Vec<topology::BridgeExpanded>>>()?;

    let mut bridges_detailed: Vec<topology::BridgeDetailed> = vec![];
    for (idx, bridge) in bridges.iter().enumerate() {
        let bridge_index = idx as u16;
        let manifest_name = bridge.name.clone();
        let bridge_name = format!("{}s{}-{}", BRIDGE_PREFIX, bridge_index, lab_id);
        let libvirt_name = format!("sherpa-bridge{}-{}-{}", bridge_index, bridge.name, lab_id);

        let mut bridge_links = vec![];
        for link in bridge.links.iter() {
            if let Some(node) = manifest_nodes.iter().find(|n| n.name == link.node) {
                let interface_idx = util::interface_to_idx(&node.model, &link.interface)?;
                bridge_links.push(topology::BridgeLinkDetailed {
                    node_name: link.node.clone(),
                    node_model: node.model,
                    interface_name: link.interface.clone(),
                    interface_index: interface_idx,
                });
            }
        }

        bridges_detailed.push(topology::BridgeDetailed {
            manifest_name,
            bridge_name,
            libvirt_name,
            index: bridge_index,
            links: bridge_links,
        });
    }

    Ok(bridges_detailed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeModel;

    // ============================================================================
    // process_manifest_nodes
    // ============================================================================

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

        let expanded = process_manifest_nodes(&nodes);

        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[0].index, 1);
        assert_eq!(expanded[0].name, "dev01");
        assert_eq!(expanded[1].index, 2);
        assert_eq!(expanded[1].name, "dev02");
        assert_eq!(expanded[2].index, 3);
        assert_eq!(expanded[2].name, "dev03");
    }

    #[test]
    fn test_process_manifest_nodes_preserves_fields() {
        let nodes = vec![topology::Node {
            name: "router01".to_string(),
            model: NodeModel::AristaVeos,
            version: Some("4.28.0".to_string()),
            memory: Some(4096),
            cpu_count: Some(2),
            privileged: Some(true),
            skip_ready_check: Some(true),
            ..Default::default()
        }];

        let expanded = process_manifest_nodes(&nodes);

        assert_eq!(expanded[0].model, NodeModel::AristaVeos);
        assert_eq!(expanded[0].version, Some("4.28.0".to_string()));
        assert_eq!(expanded[0].memory, Some(4096));
        assert_eq!(expanded[0].cpu_count, Some(2));
        assert_eq!(expanded[0].privileged, Some(true));
        assert_eq!(expanded[0].skip_ready_check, Some(true));
    }

    #[test]
    fn test_process_manifest_nodes_empty_input() {
        let nodes: Vec<topology::Node> = vec![];
        let expanded = process_manifest_nodes(&nodes);
        assert!(expanded.is_empty());
    }

    // ============================================================================
    // process_manifest_links
    // ============================================================================

    #[test]
    fn test_process_manifest_links_resolves_interfaces() {
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
        ];
        let expanded_nodes = process_manifest_nodes(&nodes);

        let links = Some(vec![topology::Link2 {
            src: "dev01::eth0".to_string(),
            dst: "dev02::eth0".to_string(),
            p2p: None,
            impairment: None,
        }]);

        let result = process_manifest_links(&links, &expanded_nodes).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].node_a, "dev01");
        assert_eq!(result[0].node_b, "dev02");
        assert_eq!(result[0].int_a, "eth0");
        assert_eq!(result[0].int_b, "eth0");
    }

    #[test]
    fn test_process_manifest_links_none_returns_empty() {
        let nodes = vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            ..Default::default()
        }];
        let expanded_nodes = process_manifest_nodes(&nodes);

        let result = process_manifest_links(&None, &expanded_nodes).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_process_manifest_links_empty_vec_returns_empty() {
        let nodes = vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            ..Default::default()
        }];
        let expanded_nodes = process_manifest_nodes(&nodes);

        let result = process_manifest_links(&Some(vec![]), &expanded_nodes).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_process_manifest_links_multiple_links() {
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
        let expanded_nodes = process_manifest_nodes(&nodes);

        let links = Some(vec![
            topology::Link2 {
                src: "dev01::eth0".to_string(),
                dst: "dev02::eth0".to_string(),
                p2p: None,
                impairment: None,
            },
            topology::Link2 {
                src: "dev02::eth1".to_string(),
                dst: "dev03::eth0".to_string(),
                p2p: None,
                impairment: None,
            },
        ]);

        let result = process_manifest_links(&links, &expanded_nodes).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].link_idx, 0);
        assert_eq!(result[1].link_idx, 1);
    }

    // ============================================================================
    // get_node_image
    // ============================================================================

    #[test]
    fn test_get_node_image_finds_default() {
        let configs = vec![
            data::NodeConfig {
                model: NodeModel::UbuntuLinux,
                version: "22.04".to_string(),
                default: false,
                ..Default::default()
            },
            data::NodeConfig {
                model: NodeModel::UbuntuLinux,
                version: "24.04".to_string(),
                default: true,
                ..Default::default()
            },
        ];

        let result = get_node_image(&NodeModel::UbuntuLinux, &configs).unwrap();
        assert_eq!(result.version, "24.04");
        assert!(result.default);
    }

    #[test]
    fn test_get_node_image_no_default_returns_error() {
        let configs = vec![data::NodeConfig {
            model: NodeModel::UbuntuLinux,
            version: "22.04".to_string(),
            default: false,
            ..Default::default()
        }];

        let result = get_node_image(&NodeModel::UbuntuLinux, &configs);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("Default node image not found"));
    }

    #[test]
    fn test_get_node_image_wrong_model_returns_error() {
        let configs = vec![data::NodeConfig {
            model: NodeModel::AristaVeos,
            version: "4.28.0".to_string(),
            default: true,
            ..Default::default()
        }];

        let result = get_node_image(&NodeModel::UbuntuLinux, &configs);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_node_image_empty_configs_returns_error() {
        let configs: Vec<data::NodeConfig> = vec![];
        let result = get_node_image(&NodeModel::UbuntuLinux, &configs);
        assert!(result.is_err());
    }

    // ============================================================================
    // process_manifest_bridges
    // ============================================================================

    #[test]
    fn test_process_manifest_bridges_generates_names() {
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
        ];
        let expanded_nodes = process_manifest_nodes(&nodes);

        let bridges = Some(vec![topology::Bridge {
            name: "mgmt".to_string(),
            links: vec!["dev01::eth0".to_string(), "dev02::eth0".to_string()],
        }]);

        let result = process_manifest_bridges(&bridges, &expanded_nodes, "abc123").unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].manifest_name, "mgmt");
        assert_eq!(result[0].bridge_name, "brs0-abc123");
        assert_eq!(result[0].libvirt_name, "sherpa-bridge0-mgmt-abc123");
        assert_eq!(result[0].index, 0);
        assert_eq!(result[0].links.len(), 2);
    }

    #[test]
    fn test_process_manifest_bridges_none_returns_empty() {
        let expanded_nodes = process_manifest_nodes(&[]);
        let result = process_manifest_bridges(&None, &expanded_nodes, "lab1").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_process_manifest_bridges_resolves_node_details() {
        let nodes = vec![topology::Node {
            name: "dev01".to_string(),
            model: NodeModel::UbuntuLinux,
            ..Default::default()
        }];
        let expanded_nodes = process_manifest_nodes(&nodes);

        let bridges = Some(vec![topology::Bridge {
            name: "lan".to_string(),
            links: vec!["dev01::eth0".to_string()],
        }]);

        let result = process_manifest_bridges(&bridges, &expanded_nodes, "xyz").unwrap();

        assert_eq!(result[0].links.len(), 1);
        assert_eq!(result[0].links[0].node_name, "dev01");
        assert_eq!(result[0].links[0].node_model, NodeModel::UbuntuLinux);
        assert_eq!(result[0].links[0].interface_name, "eth0");
    }
}
