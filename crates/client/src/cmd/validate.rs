use anyhow::{Context, Result};

use super::manifest_processing::{check_file_settings, process_manifest_nodes};
use shared::data::NodeCpuPins;
use shared::util;
use shared::validators::effective_data_interface_count;
use topology::validate::{
    check_cpu_pin_overlap, check_device_count, check_duplicate_device,
    check_duplicate_interface_link, check_interface_bounds, check_lab_description, check_lab_name,
    check_link_impairments, check_no_p2p_field, check_node_kind_settings, check_node_model_kind,
    check_node_names, check_node_ztp, check_nonzero_node_settings, check_ptx_platform,
    check_ready_timeout, check_text_file_modes, check_vm_tuning, validate_environment_variables,
    validate_provisioners, validate_tailscale, validate_talos_clusters,
};
use topology::{Manifest, process_manifest_bridges, process_manifest_links};

pub fn validate_manifest(manifest_path: &str) -> Result<()> {
    // Load manifest
    util::term_msg_underline(&format!("Loading Manifest: {manifest_path}"));
    let manifest = Manifest::load_file(manifest_path)
        .context(format!("Failed to load manifest from '{manifest_path}'"))?;

    validate_tailscale(&manifest)?;
    println!("✓ Manifest loaded successfully");
    println!();

    util::term_msg_underline("Validating Manifest");

    println!("→ Checking the lab name...");
    check_lab_name(&manifest.name)?;
    println!("  ✓ Lab name is valid");

    // Device Validators
    println!("→ Checking for duplicate device names...");
    check_node_names(&manifest.nodes)?;
    check_duplicate_device(&manifest.nodes)?;
    println!("  ✓ No duplicate devices");

    println!("→ Checking node count...");
    check_device_count(&manifest.nodes)?;
    println!("  ✓ Node count is within the lab limit");

    println!("→ Checking Talos clusters...");
    validate_talos_clusters(&manifest)?;
    println!("  ✓ Talos clusters are valid");

    println!("→ Checking provisioners...");
    validate_provisioners(&manifest)?;
    println!("  ✓ Provisioners are valid");

    println!("→ Checking the description...");
    check_lab_description(manifest.description.as_deref())?;
    println!("  ✓ Description is valid");

    println!("→ Checking settings that cannot be 0...");
    check_ready_timeout(manifest.ready_timeout)?;
    check_nonzero_node_settings(&manifest.nodes)?;
    println!("  ✓ No setting is 0");

    println!("→ Checking text files...");
    check_text_file_modes(&manifest.nodes)?;
    println!("  ✓ Text file modes are valid");

    println!("→ Checking environment variables...");
    validate_environment_variables(&manifest.nodes)?;
    println!("  ✓ Environment variable keys are valid");

    println!("→ Checking each node's kind...");
    for node in &manifest.nodes {
        check_node_model_kind(node)?;
    }
    println!("  ✓ Every node's kind is one its model takes");

    // Process manifest data (same as up.rs)
    let nodes_expanded = process_manifest_nodes(&manifest.nodes)?;
    println!("→ Checking node settings against each node's kind...");
    for node in &nodes_expanded {
        check_node_kind_settings(node)?;
    }
    check_file_settings(&manifest.nodes)?;
    println!("  ✓ Every node sets only what its kind and model take");

    println!("→ Checking links and bridges...");
    let links = manifest.links.as_deref().unwrap_or_default();
    let bridges = manifest.bridges.as_deref().unwrap_or_default();
    check_no_p2p_field(links)?;
    check_link_impairments(links)?;
    let links_detailed = process_manifest_links(links, &nodes_expanded)?;
    let bridges_detailed = process_manifest_bridges(bridges, &nodes_expanded, "validate")?;
    println!("  ✓ All linked and bridged devices exist");

    // Per-node validators
    println!("→ Checking interface configurations...");
    for node in &nodes_expanded {
        // Without a server, a node's image is its model and kind's built-in settings.
        let node_image = node.model_kind.node_config();

        let data_interface_count =
            effective_data_interface_count(&node.name, node.interfaces.data_count, &node_image)?;

        // Interface bounds check
        check_interface_bounds(
            &node.name,
            &node_image.model,
            data_interface_count,
            node_image.interfaces.reserved_count,
            &links_detailed,
            &bridges_detailed,
        )?;
    }
    println!("  ✓ All interface configurations valid");

    println!("→ Checking VM tuning...");
    for node in &nodes_expanded {
        let node_image = node.model_kind.node_config();
        check_vm_tuning(node, &node_image)?;
        check_ptx_platform(node, &node_image)?;
        check_node_ztp(node)?;
    }
    let pins: Vec<NodeCpuPins> = nodes_expanded
        .iter()
        .map(|node| NodeCpuPins::of(&node.name, &node.cpu, &node.boot_disk))
        .filter(|pins| !pins.is_empty())
        .collect();
    check_cpu_pin_overlap(&pins)?;
    println!("  ✓ VM tuning is valid");

    // Connection validators
    if !links_detailed.is_empty() || !bridges_detailed.is_empty() {
        println!("→ Checking link configurations...");
        check_duplicate_interface_link(&links_detailed, &bridges_detailed)?;
        println!("  ✓ No duplicate interface usage");
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
    fn validate_command_003_a_link_past_the_built_in_data_count_is_taken_once_data_count_is_set() {
        let check = |name: &str, interfaces: &str| {
            let manifest = format!(
                r#"
name = "override-lab"

nodes = [
  {{ name = "dev01", model = "ubuntu_linux"{interfaces} }},
  {{ name = "dev02", model = "ubuntu_linux"{interfaces} }},
]

links = [
  {{ src = "dev01::eth4", dst = "dev02::eth4" }},
]
"#
            );
            let path = write_temp_manifest(name, &manifest);
            let result = validate_manifest(path.to_str().expect("temp path is utf-8"));
            fs::remove_file(path).ok();
            result
        };

        assert!(check("override-fail", "").is_err());
        assert!(check("override-pass", r", interfaces = { data_count = 4 }").is_ok());
    }

    #[test]
    fn boot_settings_008_sherpa_validate_refuses_an_environment_variable_key_not_a_name() {
        let path = write_temp_manifest(
            "env-key",
            r#"
name = "env-lab"

nodes = [
  { name = "dev01", model = "ubuntu_linux", environment_variables = ["BAD-KEY=value"] },
]
"#,
        );
        let result = validate_manifest(path.to_str().expect("temp path is utf-8"));
        fs::remove_file(path).ok();

        let message = format!("{:#}", result.unwrap_err());
        assert!(
            message.contains("key 'BAD-KEY' for node 'dev01'"),
            "{message}"
        );
    }
}
