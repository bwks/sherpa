use anyhow::{Context, Result};

use shared::data::{NodeConfig, NodeModel, Ztp};
use shared::validators::{check_ztp, effective_ztp};
use topology::{NodeExpanded, TalosCluster, find_talos_cluster};

use crate::services::error::ServiceError;

/// Check a node's ZTP before anything is created or torn down, and return the setting it runs
/// with: the manifest's or else its image's, which its model must take, and which must be able
/// to use the node's `ztp.config` and, for a Talos cluster member, the machine config Sherpa
/// generates for it.
///
/// # Errors
///
/// Fails when the setting is a method the model does not take, when the node sets a
/// `ztp.config` its setting would not use, and when a Talos cluster member has ZTP off.
pub fn validate_node_ztp(
    node: &NodeExpanded,
    image: &NodeConfig,
    talos_clusters: &[TalosCluster],
) -> Result<Ztp> {
    let ztp = effective_ztp(node.ztp.method, image);
    check_ztp(node.model_kind, ztp)
        .with_context(|| format!("ZTP validation failed for node: {}", node.name))
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    if let Some((cluster, _)) = find_talos_cluster(talos_clusters, &node.name)
        && ztp == Ztp::Disabled
    {
        return Err(ServiceError::Invalid(format!(
            "Node '{}' joins talos cluster '{}' but its ZTP is disabled, so the machine config \
             Sherpa generates for it would not reach it",
            node.name, cluster.name
        ))
        .into());
    }
    if node.ztp.config.is_some() {
        check_ztp_config_is_used(&node.name, node.model_kind.model(), ztp)?;
    }
    Ok(ztp)
}

/// Whether a node's own `ztp.config` is used with its ZTP setting. The methods that deliver a
/// config file take it in place of the generated one. Cloud-init, Ignition and the HTTP
/// config of `sonic_linux` are built from the manifest's other fields, and with ZTP off nothing delivers it;
/// the one exception is Vault, whose `ztp.config` is its own `config.hcl`, a start setting.
fn check_ztp_config_is_used(node_name: &str, model: NodeModel, ztp: Ztp) -> Result<()> {
    match ztp {
        Ztp::Tftp | Ztp::Cdrom | Ztp::Disk | Ztp::Usb | Ztp::NoCloud | Ztp::Volume => Ok(()),
        Ztp::Disabled if model == NodeModel::HashicorpVault => Ok(()),
        Ztp::Disabled => Err(ServiceError::Invalid(format!(
            "Node '{node_name}' sets ztp.config, but its ZTP is disabled, so the config would \
             not be used"
        ))
        .into()),
        Ztp::CloudInit | Ztp::Ignition | Ztp::Http => Err(ServiceError::Invalid(format!(
            "Node '{node_name}' sets ztp.config, which ZTP method '{ztp}' does not use. Only \
             tftp, cdrom, disk, usb, nocloud and volume take a custom ZTP config"
        ))
        .into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{KindSettings, ModelKind};
    use topology::NodeZtp;

    fn node(model: NodeModel, ztp: Option<Ztp>, ztp_config: Option<&str>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_owned(),
            model_kind: ModelKind::default_for(model),
            ztp: NodeZtp {
                method: ztp,
                config: ztp_config.map(str::to_owned),
            },
            ..NodeExpanded::default()
        }
    }

    fn validate(model: NodeModel, ztp: Option<Ztp>, ztp_config: Option<&str>) -> Result<Ztp> {
        validate_node_ztp(
            &node(model, ztp, ztp_config),
            &ModelKind::default_for(model).node_config(),
            &[],
        )
    }

    #[test]
    fn ztp_setting_002_a_node_is_built_with_its_manifests_method_or_else_its_images() {
        assert_eq!(
            validate(NodeModel::FrrLinux, None, None).unwrap(),
            Ztp::Volume
        );
        assert_eq!(
            validate(NodeModel::FrrLinux, Some(Ztp::Disabled), None).unwrap(),
            Ztp::Disabled
        );
        assert_eq!(
            validate(NodeModel::JuniperEvolved, Some(Ztp::Usb), None).unwrap(),
            Ztp::Usb
        );
    }

    #[test]
    fn ztp_setting_005_up_and_redeploy_refuse_an_images_method_as_a_manifests() {
        let error = validate(NodeModel::FrrLinux, Some(Ztp::Cdrom), None).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("node1"), "{error}");
        assert!(error.contains("allows: disabled, volume"), "{error}");

        // An image whose stored setting its model does not take, however it got there.
        let mut image = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        let KindSettings::VirtualMachine(vm) = &mut image.settings else {
            panic!("ubuntu_linux is a VM");
        };
        vm.ztp.method = Ztp::Volume;
        let error =
            validate_node_ztp(&node(NodeModel::UbuntuLinux, None, None), &image, &[]).unwrap_err();
        assert!(format!("{error:#}").contains("allows: disabled, cloud-init"));
    }

    #[test]
    fn custom_config_005_a_config_is_taken_only_by_the_methods_that_deliver_one() {
        for (model, ztp) in [
            (NodeModel::JuniperEvolved, Ztp::Tftp),
            (NodeModel::JuniperEvolved, Ztp::Usb),
            (NodeModel::JuniperSrxv3, Ztp::Cdrom),
            (NodeModel::FrrLinux, Ztp::Volume),
            (NodeModel::TalosLinux, Ztp::NoCloud),
        ] {
            assert_eq!(
                validate(model, Some(ztp), Some("config")).unwrap(),
                ztp,
                "{model} with {ztp}"
            );
        }
        for (model, ztp) in [
            (NodeModel::UbuntuLinux, Ztp::CloudInit),
            (NodeModel::FlatcarLinux, Ztp::Ignition),
        ] {
            let error = validate(model, Some(ztp), Some("config"))
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("which ZTP method '{ztp}' does not use")),
                "{model}: {error}"
            );
            assert!(validate(model, Some(ztp), None).is_ok(), "{model}");
        }
    }

    #[test]
    fn custom_config_006_a_config_with_ztp_off_is_refused_except_for_vault() {
        for model in NodeModel::to_vec() {
            let result = validate(model, Some(Ztp::Disabled), Some("config"));
            if model == NodeModel::HashicorpVault {
                assert_eq!(result.unwrap(), Ztp::Disabled);
            } else {
                let error = result.unwrap_err().to_string();
                assert!(error.contains("its ZTP is disabled"), "{model}: {error}");
            }
        }
    }

    #[test]
    fn ztp_setting_014_a_talos_cluster_member_with_ztp_off_is_refused() {
        let clusters = [TalosCluster {
            name: "prod".to_owned(),
            controllers: vec!["node1".to_owned()],
            ..TalosCluster::default()
        }];
        let image = ModelKind::default_for(NodeModel::TalosLinux).node_config();
        let member = node(NodeModel::TalosLinux, Some(Ztp::Disabled), None);
        let error = validate_node_ztp(&member, &image, &clusters)
            .unwrap_err()
            .to_string();
        assert!(error.contains("talos cluster 'prod'"), "{error}");

        let member = node(NodeModel::TalosLinux, None, None);
        assert_eq!(
            validate_node_ztp(&member, &image, &clusters).unwrap(),
            Ztp::NoCloud
        );
        // Outside a cluster, a Talos node may run without a config.
        let alone = node(NodeModel::TalosLinux, Some(Ztp::Disabled), None);
        assert_eq!(
            validate_node_ztp(&alone, &image, &[]).unwrap(),
            Ztp::Disabled
        );
    }
}
