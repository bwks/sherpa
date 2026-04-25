use anyhow::{Context, Result};

use super::manifest_processing::{
    get_node_image, process_manifest_bridges, process_manifest_links, process_manifest_nodes,
};
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
    let node_images: Vec<NodeConfig> = all_models.into_iter().map(NodeConfig::get_model).collect();

    util::term_msg_underline("Validating Manifest");

    // Device Validators
    println!("→ Checking for duplicate device names...");
    validate::check_duplicate_device(&manifest.nodes)?;
    println!("  ✓ No duplicate devices");

    // Process manifest data (same as up.rs)
    let nodes_expanded = process_manifest_nodes(&manifest.nodes);
    let links_detailed = process_manifest_links(&manifest.links, &nodes_expanded)?;
    let bridges_detailed =
        process_manifest_bridges(&manifest.bridges, &nodes_expanded, "validate")?;

    // Per-node validators
    println!("→ Checking interface configurations...");
    for node in &nodes_expanded {
        let node_image = get_node_image(&node.model, &node_images)?;

        // Management interface check
        if !node_image.dedicated_management_interface {
            validate::check_mgmt_usage(&node.name, 0, &links_detailed, &bridges_detailed)?;
        }

        let data_interface_count = validate::effective_data_interface_count(
            &node.name,
            node.data_interface_count,
            &node_image,
        )?;

        // Interface bounds check
        validate::check_interface_bounds(
            &node.name,
            &node_image.model,
            data_interface_count,
            node_image.reserved_interface_count,
            node_image.dedicated_management_interface,
            &links_detailed,
            &bridges_detailed,
        )?;
    }
    println!("  ✓ All interface configurations valid");

    // Connection validators
    if !links_detailed.is_empty() || !bridges_detailed.is_empty() {
        println!("→ Checking link configurations...");
        validate::check_duplicate_interface_link(&links_detailed, &bridges_detailed)?;
        println!("  ✓ No duplicate interface usage");

        validate::check_link_device(&manifest.nodes, &links_detailed)?;
        println!("  ✓ All linked devices exist");
    }

    // Bridge validators
    if !bridges_detailed.is_empty() {
        println!("→ Checking bridge configurations...");
        validate::check_bridge_device(&manifest.nodes, &bridges_detailed)?;
        println!("  ✓ All bridge devices exist");
    }

    println!();
    println!("{}", util::emoji_success("Manifest validation passed!"));

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn write_temp_manifest(name: &str, contents: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("sherpa-{name}-{}.toml", std::process::id()));
        fs::write(&path, contents).expect("writes temp manifest");
        path
    }

    #[test]
    fn test_validate_manifest_accepts_data_interface_count_override() {
        let manifest = r#"
name = "override-lab"

nodes = [
  { name = "dev01", model = "ubuntu_linux", data_interface_count = 4 },
  { name = "dev02", model = "ubuntu_linux", data_interface_count = 4 },
]

links = [
  { src = "dev01::eth4", dst = "dev02::eth4" },
]
"#;
        let path = write_temp_manifest("override-pass", manifest);
        let result = validate_manifest(path.to_str().expect("temp path is utf-8"));
        fs::remove_file(path).ok();
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_manifest_rejects_link_past_default_interface_count() {
        let manifest = r#"
name = "override-lab"

nodes = [
  { name = "dev01", model = "ubuntu_linux" },
  { name = "dev02", model = "ubuntu_linux" },
]

links = [
  { src = "dev01::eth2", dst = "dev02::eth2" },
]
"#;
        let path = write_temp_manifest("override-fail", manifest);
        let result = validate_manifest(path.to_str().expect("temp path is utf-8"));
        fs::remove_file(path).ok();
        assert!(result.is_err());
    }
}
