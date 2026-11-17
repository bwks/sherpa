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
    if image.model != NodeModel::JuniperEvolved || image.kind != NodeKind::Container {
        if platform.is_some() {
            bail!(
                "Manifest - node: '{}' sets `container.platform`, which is only for a juniper_evolved container, not {} ({})",
                node.name,
                image.model,
                image.kind
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
    fn a_platform_on_cevolved_is_accepted() {
        let image = cevolved().node_config();
        check_ptx_platform(&node(cevolved(), None), &image).unwrap();
        check_ptx_platform(&node(cevolved(), Some(PtxPlatform::Bx)), &image).unwrap();
    }

    #[test]
    fn a_platform_on_another_model_is_refused() {
        let image = ModelKind::default_for(NodeModel::NokiaSrlinux).node_config();
        let error = check_ptx_platform(
            &node(
                ModelKind::default_for(NodeModel::NokiaSrlinux),
                Some(PtxPlatform::Bt),
            ),
            &image,
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("only for a juniper_evolved container"),
            "{error}"
        );
        check_ptx_platform(
            &node(ModelKind::default_for(NodeModel::NokiaSrlinux), None),
            &image,
        )
        .unwrap();
    }

    #[test]
    fn a_platform_on_a_juniper_evolved_virtual_machine_is_refused() {
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
    fn bt_takes_12_data_interfaces_and_bx_36() {
        let image = cevolved().node_config();
        let mut bt = node(cevolved(), None);
        bt.interfaces.data_count = Some(13);
        let error = check_ptx_platform(&bt, &image).unwrap_err();
        assert!(
            error.to_string().contains("platform bt has at most 12"),
            "{error}"
        );

        let mut bx = node(cevolved(), Some(PtxPlatform::Bx));
        bx.interfaces.data_count = Some(36);
        check_ptx_platform(&bx, &image).unwrap();
    }

    #[test]
    fn fewer_vcpus_or_less_memory_than_cevolved_needs_is_refused() {
        let image = cevolved().node_config();
        let mut small_cpu = node(cevolved(), None);
        small_cpu.cpu.count = Some(2);
        assert!(check_ptx_platform(&small_cpu, &image).is_err());

        let mut small_memory = node(cevolved(), None);
        small_memory.memory = Some(4096);
        assert!(check_ptx_platform(&small_memory, &image).is_err());

        let mut large = node(cevolved(), None);
        large.cpu.count = Some(8);
        large.memory = Some(16384);
        check_ptx_platform(&large, &image).unwrap();
    }

    #[test]
    fn cevolved_memory_must_leave_room_for_the_container_overhead() {
        let image = cevolved().node_config();
        let mut largest = node(cevolved(), None);
        largest.memory = Some(u16::MAX - JUNIPER_EVOLVED_CONTAINER_OVERHEAD_MIB);
        check_ptx_platform(&largest, &image).unwrap();
        let mut too_large = node(cevolved(), None);
        too_large.memory = Some(u16::MAX);
        let error = check_ptx_platform(&too_large, &image)
            .unwrap_err()
            .to_string();
        assert!(error.contains("at most"), "{error}");
    }
}
