use anyhow::{Result, bail};

use crate::NodeExpanded;
use shared::data::{NodeConfig, NodeModel};

/// Check a node's `platform` and, for `juniper_cevolved`, that its data interfaces fit the
/// platform and its vCPUs and memory meet cJunosEvolved's minimums.
///
/// # Errors
///
/// Fails when `platform` is set on another model, or the node does not fit its platform.
pub fn check_ptx_platform(node: &NodeExpanded, image: &NodeConfig) -> Result<()> {
    if image.model != NodeModel::JuniperCevolved {
        if node.platform.is_some() {
            bail!(
                "Manifest - node: '{}' sets `platform`, which is only for juniper_cevolved, not {}",
                node.name,
                image.model
            );
        }
        return Ok(());
    }
    let platform = node.platform.unwrap_or_default();
    let interfaces = node
        .data_interface_count
        .unwrap_or(image.interfaces.data_count);
    if interfaces > platform.max_data_interfaces() {
        bail!(
            "Manifest - node: '{}' has {} data interfaces, but platform {} has at most {}",
            node.name,
            interfaces,
            platform,
            platform.max_data_interfaces()
        );
    }
    let cpu_count = node.cpu_count.unwrap_or(image.compute.cpu_count);
    if cpu_count < image.compute.cpu_count {
        bail!(
            "Manifest - node: '{}' has {} vCPUs; juniper_cevolved needs at least {}",
            node.name,
            cpu_count,
            image.compute.cpu_count
        );
    }
    let memory = node.memory.unwrap_or(image.compute.memory);
    if memory < image.compute.memory {
        bail!(
            "Manifest - node: '{}' has {} MB of memory; juniper_cevolved needs at least {}",
            node.name,
            memory,
            image.compute.memory
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::PtxPlatform;

    fn node(model: NodeModel, platform: Option<PtxPlatform>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model,
            platform,
            ..NodeExpanded::default()
        }
    }

    #[test]
    fn a_platform_on_cevolved_is_accepted() {
        let image = NodeConfig::get_model(NodeModel::JuniperCevolved);
        check_ptx_platform(&node(NodeModel::JuniperCevolved, None), &image).unwrap();
        check_ptx_platform(
            &node(NodeModel::JuniperCevolved, Some(PtxPlatform::Bx)),
            &image,
        )
        .unwrap();
    }

    #[test]
    fn a_platform_on_another_model_is_refused() {
        let image = NodeConfig::get_model(NodeModel::NokiaSrlinux);
        let error = check_ptx_platform(
            &node(NodeModel::NokiaSrlinux, Some(PtxPlatform::Bt)),
            &image,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("only for juniper_cevolved"),
            "{error}"
        );
        check_ptx_platform(&node(NodeModel::NokiaSrlinux, None), &image).unwrap();
    }

    #[test]
    fn bt_takes_12_data_interfaces_and_bx_36() {
        let image = NodeConfig::get_model(NodeModel::JuniperCevolved);
        let mut bt = node(NodeModel::JuniperCevolved, None);
        bt.data_interface_count = Some(13);
        let error = check_ptx_platform(&bt, &image).unwrap_err();
        assert!(
            error.to_string().contains("platform bt has at most 12"),
            "{error}"
        );

        let mut bx = node(NodeModel::JuniperCevolved, Some(PtxPlatform::Bx));
        bx.data_interface_count = Some(36);
        check_ptx_platform(&bx, &image).unwrap();
    }

    #[test]
    fn fewer_vcpus_or_less_memory_than_cevolved_needs_is_refused() {
        let image = NodeConfig::get_model(NodeModel::JuniperCevolved);
        let mut small_cpu = node(NodeModel::JuniperCevolved, None);
        small_cpu.cpu_count = Some(2);
        assert!(check_ptx_platform(&small_cpu, &image).is_err());

        let mut small_memory = node(NodeModel::JuniperCevolved, None);
        small_memory.memory = Some(4096);
        assert!(check_ptx_platform(&small_memory, &image).is_err());

        let mut large = node(NodeModel::JuniperCevolved, None);
        large.cpu_count = Some(8);
        large.memory = Some(16384);
        check_ptx_platform(&large, &image).unwrap();
    }
}
