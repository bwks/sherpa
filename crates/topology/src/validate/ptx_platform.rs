use anyhow::{Result, bail};

use crate::NodeExpanded;
use shared::data::{NodeConfig, NodeModel};
use shared::konst::JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB;

/// Check a node's `container.platform` and, for `juniper_cevolved`, that its data interfaces fit the
/// platform, its vCPUs and memory meet cJunosEvolved's minimums, and its memory leaves room for
/// the container's overhead in a node's memory field.
///
/// # Errors
///
/// Fails when `container.platform` is set on another model, or the node does not fit its platform.
pub fn check_ptx_platform(node: &NodeExpanded, image: &NodeConfig) -> Result<()> {
    let platform = node
        .container
        .as_ref()
        .and_then(|container| container.platform);
    if image.model != NodeModel::JuniperCevolved {
        if platform.is_some() {
            bail!(
                "Manifest - node: '{}' sets `container.platform`, which is only for juniper_cevolved, not {}",
                node.name,
                image.model
            );
        }
        return Ok(());
    }
    let platform = platform.unwrap_or_default();
    let interfaces = node
        .interfaces
        .data_count
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
    let cpu_count = node.cpu.count.unwrap_or(image.cpu.count);
    if cpu_count < image.cpu.count {
        bail!(
            "Manifest - node: '{}' has {} vCPUs; juniper_cevolved needs at least {}",
            node.name,
            cpu_count,
            image.cpu.count
        );
    }
    let memory = node.memory.unwrap_or(image.memory);
    if memory < image.memory {
        bail!(
            "Manifest - node: '{}' has {} MB of memory; juniper_cevolved needs at least {}",
            node.name,
            memory,
            image.memory
        );
    }
    let largest = u16::MAX - JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB;
    if memory > largest {
        bail!(
            "Manifest - node: '{}' has {} MB of memory; juniper_cevolved takes at most {}, as its \
             container needs {} MB more",
            node.name,
            memory,
            largest,
            JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeContainer;
    use shared::data::PtxPlatform;

    fn node(model: NodeModel, platform: Option<PtxPlatform>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model,
            container: Some(NodeContainer {
                platform,
                ..NodeContainer::default()
            }),
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
        bt.interfaces.data_count = Some(13);
        let error = check_ptx_platform(&bt, &image).unwrap_err();
        assert!(
            error.to_string().contains("platform bt has at most 12"),
            "{error}"
        );

        let mut bx = node(NodeModel::JuniperCevolved, Some(PtxPlatform::Bx));
        bx.interfaces.data_count = Some(36);
        check_ptx_platform(&bx, &image).unwrap();
    }

    #[test]
    fn fewer_vcpus_or_less_memory_than_cevolved_needs_is_refused() {
        let image = NodeConfig::get_model(NodeModel::JuniperCevolved);
        let mut small_cpu = node(NodeModel::JuniperCevolved, None);
        small_cpu.cpu.count = Some(2);
        assert!(check_ptx_platform(&small_cpu, &image).is_err());

        let mut small_memory = node(NodeModel::JuniperCevolved, None);
        small_memory.memory = Some(4096);
        assert!(check_ptx_platform(&small_memory, &image).is_err());

        let mut large = node(NodeModel::JuniperCevolved, None);
        large.cpu.count = Some(8);
        large.memory = Some(16384);
        check_ptx_platform(&large, &image).unwrap();
    }

    #[test]
    fn cevolved_memory_must_leave_room_for_the_container_overhead() {
        let image = NodeConfig::get_model(NodeModel::JuniperCevolved);
        let mut largest = node(NodeModel::JuniperCevolved, None);
        largest.memory = Some(u16::MAX - JUNIPER_CEVOLVED_CONTAINER_OVERHEAD_MIB);
        check_ptx_platform(&largest, &image).unwrap();
        let mut too_large = node(NodeModel::JuniperCevolved, None);
        too_large.memory = Some(u16::MAX);
        let error = check_ptx_platform(&too_large, &image)
            .unwrap_err()
            .to_string();
        assert!(error.contains("at most"), "{error}");
    }
}
