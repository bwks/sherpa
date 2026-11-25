use anyhow::{Context, Result, bail};

use shared::data::{NodeConfig, Ztp};
use shared::error::SharedError;
use shared::validators::{check_ztp, effective_ztp};
use topology::validate::check_node_ztp_config;
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
    if let Err(error) = check_ztp(node.model_kind, ztp) {
        let refusal = error
            .downcast::<SharedError>()
            .context("Failed to check the node's ZTP setting")?;
        bail!(ServiceError::NodeZtpUnsupported {
            node: node.name.clone(),
            refusal,
        });
    }
    if let Some((cluster, _)) = find_talos_cluster(talos_clusters, &node.name)
        && ztp == Ztp::Disabled
    {
        bail!(ServiceError::TalosMemberZtpDisabled {
            node: node.name.clone(),
            cluster: cluster.name.clone(),
        });
    }
    check_node_ztp_config(node, ztp)
        .map_err(|error| ServiceError::Invalid(format!("{error:#}")))?;
    Ok(ztp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::{KindSettings, ModelKind, NodeModel};
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
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::NodeZtpUnsupported {
                node: "node1".to_owned(),
                refusal: SharedError::UnsupportedZtp {
                    ztp: Ztp::Cdrom,
                    model: NodeModel::FrrLinux,
                    allowed: vec![Ztp::Disabled, Ztp::Volume],
                },
            })
        );

        // An image whose stored setting its model does not take, however it got there.
        let mut image = ModelKind::default_for(NodeModel::UbuntuLinux).node_config();
        let KindSettings::VirtualMachine(vm) = &mut image.settings else {
            panic!("ubuntu_linux is a VM");
        };
        vm.ztp.method = Ztp::Volume;
        let error =
            validate_node_ztp(&node(NodeModel::UbuntuLinux, None, None), &image, &[]).unwrap_err();
        assert!(
            matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::NodeZtpUnsupported {
                    refusal: SharedError::UnsupportedZtp { allowed, .. },
                    ..
                }) if allowed == &[Ztp::Disabled, Ztp::CloudInit]
            ),
            "{error:#}"
        );
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
        let error = validate_node_ztp(&member, &image, &clusters).unwrap_err();
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::TalosMemberZtpDisabled {
                node: "node1".to_owned(),
                cluster: "prod".to_owned(),
            })
        );

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
