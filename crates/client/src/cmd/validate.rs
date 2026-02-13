use anyhow::{Context, Result};

use super::manifest_processing::{get_node_config, process_manifest_links, process_manifest_nodes};
use shared::data::{NodeConfig, NodeModel};
use shared::util;
use topology::Manifest;

pub fn validate_manifest(manifest_path: &str) -> Result<()> {
    // Load manifest
    util::term_msg_underline(&format!("Loading Manifest: {}", manifest_path));
    let manifest = Manifest::load_file(manifest_path)
        .context(format!("Failed to load manifest from '{}'", manifest_path))?;

    println!("✓ Manifest loaded successfully");
    println!();

    // Get all node configs (without database)
    let all_models = NodeModel::to_vec();
    let node_configs: Vec<NodeConfig> = all_models.into_iter().map(NodeConfig::get_model).collect();

    util::term_msg_underline("Validating Manifest");

    // Device Validators
    println!("→ Checking for duplicate device names...");
    validate::check_duplicate_device(&manifest.nodes)?;
    println!("  ✓ No duplicate devices");

    // Process manifest data (same as up.rs)
    let nodes_expanded = process_manifest_nodes(&manifest.nodes);
    let links_detailed = process_manifest_links(&manifest.links, &nodes_expanded)?;

    // Per-node validators
    println!("→ Checking interface configurations...");
    for node in &nodes_expanded {
        let node_config = get_node_config(&node.model, &node_configs)?;

        // Management interface check
        if !node_config.dedicated_management_interface {
            validate::check_mgmt_usage(&node.name, 0, &links_detailed)?;
        }

        // Interface bounds check
        validate::check_interface_bounds(
            &node.name,
            &node_config.model,
            node_config.data_interface_count,
            node_config.reserved_interface_count,
            node_config.dedicated_management_interface,
            &links_detailed,
        )?;
    }
    println!("  ✓ All interface configurations valid");

    // Connection validators
    if !links_detailed.is_empty() {
        println!("→ Checking link configurations...");
        validate::check_duplicate_interface_link(&links_detailed)?;
        println!("  ✓ No duplicate interface usage");

        validate::check_link_device(&manifest.nodes, &links_detailed)?;
        println!("  ✓ All linked devices exist");
    }

    println!();
    println!("✅ Manifest validation passed!");

    Ok(())
}
