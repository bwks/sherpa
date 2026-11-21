use anyhow::{Result, bail};

use crate::NodeExpanded;
use shared::data::{NodeConfig, NodeKind, NodeModel};
use shared::konst::JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB;

/// Check a node's `container.platform` and, for a `juniper_evolved` container, that its data
/// interfaces fit the platform, its vCPUs and memory meet cJunosEvolved's minimums, and its memory leaves room for
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
    if image.model != NodeModel::JuniperEvolved || image.kind() != NodeKind::Container {
        if platform.is_some() {
            bail!(
                "Manifest - node: '{}' sets `container.platform`, which is only for a juniper_evolved container, not {} ({})",
                node.name,
                image.model,
                image.kind()
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
            "Manifest - node: '{}' has {} vCPUs; a juniper_evolved container needs at least {}",
            node.name,
            cpu_count,
            image.cpu.count
        );
    }
    let memory = node.memory.unwrap_or(image.memory);
    if memory < image.memory {
        bail!(
            "Manifest - node: '{}' has {} MB of memory; a juniper_evolved container needs at least {}",
            node.name,
            memory,
            image.memory
        );
    }
    let largest = u16::MAX - JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB;
    if memory > largest {
        bail!(
            "Manifest - node: '{}' has {} MB of memory; a juniper_evolved container takes at most {}, as its \
             container needs {} MB more",
            node.name,
            memory,
            largest,
            JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeContainer;
    use shared::data::{ModelKind, PtxPlatform};

    fn cevolved() -> ModelKind {
        ModelKind::new(NodeModel::JuniperEvolved, Some(NodeKind::Container)).unwrap()
    }

    fn node(model_kind: ModelKind, platform: Option<PtxPlatform>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_string(),
            model_kind,
            container: Some(NodeContainer {
                platform,
                ..NodeContainer::default()
            }),
            ..NodeExpanded::default()
        }
    }

    #[test]
    fn node_settings_006_a_platform_on_a_node_that_is_not_a_juniper_evolved_container_is_refused() {
        let srlinux = ModelKind::default_for(NodeModel::NokiaSrlinux);
        let error = check_ptx_platform(
            &node(srlinux, Some(PtxPlatform::Bt)),
            &srlinux.node_config(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("only for a juniper_evolved container, not nokia_srlinux"),
            "{error}"
        );
        check_ptx_platform(&node(srlinux, None), &srlinux.node_config()).unwrap();

        let vm = ModelKind::default_for(NodeModel::JuniperEvolved);
        let error = check_ptx_platform(&node(vm, Some(PtxPlatform::Bt)), &vm.node_config())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(
                "only for a juniper_evolved container, not juniper_evolved (virtual_machine)"
            ),
            "{error}"
        );
    }

    #[test]
    fn node_settings_007_a_juniper_evolved_container_takes_the_data_interfaces_of_its_platform() {
        let image = cevolved().node_config();
        for (platform, most) in [
            (None, 12),
            (Some(PtxPlatform::Bt), 12),
            (Some(PtxPlatform::Bx), 36),
        ] {
            let mut fits = node(cevolved(), platform);
            fits.interfaces.data_count = Some(most);
            check_ptx_platform(&fits, &image).unwrap();

            let mut over = node(cevolved(), platform);
            over.interfaces.data_count = Some(most + 1);
            let error = check_ptx_platform(&over, &image).unwrap_err().to_string();
            assert!(error.contains(&format!("has at most {most}")), "{error}");
        }
    }

    #[test]
    fn node_settings_008_a_juniper_evolved_container_smaller_than_its_image_is_refused() {
        let image = cevolved().node_config();
        let mut small_cpu = node(cevolved(), None);
        small_cpu.cpu.count = Some(2);
        let error = check_ptx_platform(&small_cpu, &image)
            .unwrap_err()
            .to_string();
        assert!(error.contains("has 2 vCPUs"), "{error}");

        let mut small_memory = node(cevolved(), None);
        small_memory.memory = Some(4096);
        let error = check_ptx_platform(&small_memory, &image)
            .unwrap_err()
            .to_string();
        assert!(error.contains("has 4096 MB of memory"), "{error}");

        let mut large = node(cevolved(), None);
        large.cpu.count = Some(8);
        large.memory = Some(16384);
        check_ptx_platform(&large, &image).unwrap();
    }

    #[test]
    fn node_settings_009_a_juniper_evolved_containers_memory_is_at_most_63487_mb() {
        let image = cevolved().node_config();
        let mut largest = node(cevolved(), None);
        largest.memory = Some(63_487);
        check_ptx_platform(&largest, &image).unwrap();
        let mut too_large = node(cevolved(), None);
        too_large.memory = Some(63_488);
        let error = check_ptx_platform(&too_large, &image)
            .unwrap_err()
            .to_string();
        assert!(error.contains("at most 63487"), "{error}");
    }
}
