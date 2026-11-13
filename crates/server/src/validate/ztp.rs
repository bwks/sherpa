use anyhow::{Context, Result, bail};

use shared::data::{NodeConfig, NodeModel, Ztp};
use shared::validators::{check_ztp, effective_ztp};
use topology::{NodeExpanded, TalosCluster, find_talos_cluster};

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
    check_ztp(node.model, ztp)
        .with_context(|| format!("ZTP validation failed for node: {}", node.name))?;
    if let Some((cluster, _)) = find_talos_cluster(talos_clusters, &node.name)
        && ztp == Ztp::Disabled
    {
        bail!(
            "Node '{}' joins talos cluster '{}' but its ZTP is disabled, so the machine config \
             Sherpa generates for it would not reach it",
            node.name,
            cluster.name
        );
    }
    if node.ztp.config.is_some() {
        check_ztp_config_is_used(&node.name, node.model, ztp)?;
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
        Ztp::Disabled => bail!(
            "Node '{node_name}' sets ztp.config, but its ZTP is disabled, so the config would \
             not be used"
        ),
        Ztp::CloudInit | Ztp::Ignition | Ztp::Http => bail!(
            "Node '{node_name}' sets ztp.config, which ZTP method '{ztp}' does not use. Only \
             tftp, cdrom, disk, usb, nocloud and volume take a custom ZTP config"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use topology::NodeZtp;

    fn node(model: NodeModel, ztp: Option<Ztp>, ztp_config: Option<&str>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_owned(),
            model,
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
            &NodeConfig::get_model(model),
            &[],
        )
    }

    #[test]
    fn the_images_setting_is_used_unless_the_node_sets_one() {
        assert_eq!(
            validate(NodeModel::FrrLinux, None, None).unwrap(),
            Ztp::Volume
        );
        assert_eq!(
            validate(NodeModel::FrrLinux, Some(Ztp::Disabled), None).unwrap(),
            Ztp::Disabled
        );
        assert_eq!(
            validate(NodeModel::JuniperVsrxv3, Some(Ztp::Tftp), None).unwrap(),
            Ztp::Tftp
        );
    }

    #[test]
    fn a_method_the_model_does_not_take_is_refused_from_either_source() {
        let error = validate(NodeModel::FrrLinux, Some(Ztp::Cdrom), None).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("node1"), "{error}");
        assert!(error.contains("allows: disabled, volume"), "{error}");

        // An image whose stored setting its model does not take, however it got there.
        let mut image = NodeConfig::get_model(NodeModel::UbuntuLinux);
        image.ztp.method = Ztp::Volume;
        let error =
            validate_node_ztp(&node(NodeModel::UbuntuLinux, None, None), &image, &[]).unwrap_err();
        assert!(format!("{error:#}").contains("allows: disabled, cloud-init"));
    }

    #[test]
    fn a_config_is_accepted_only_where_a_generator_reads_it() {
        for model in NodeModel::to_vec() {
            let settings = model.ztp_methods().iter().copied().chain([Ztp::Disabled]);
            for ztp in settings {
                let read = match ztp {
                    Ztp::Tftp | Ztp::Cdrom | Ztp::Disk | Ztp::Usb | Ztp::NoCloud | Ztp::Volume => {
                        true
                    }
                    Ztp::Disabled => model == NodeModel::HashicorpVault,
                    Ztp::CloudInit | Ztp::Ignition | Ztp::Http => false,
                };
                let result = validate(model, Some(ztp), Some("config"));
                assert_eq!(result.is_ok(), read, "{model} with {ztp}: {result:?}");
                assert!(
                    validate(model, Some(ztp), None).is_ok(),
                    "{model} with {ztp}"
                );
            }
        }
    }

    #[test]
    fn a_config_on_a_node_with_ztp_off_is_refused_except_vaults() {
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
    fn a_talos_cluster_member_with_ztp_off_is_refused() {
        let clusters = [TalosCluster {
            name: "prod".to_owned(),
            controllers: vec!["node1".to_owned()],
            ..TalosCluster::default()
        }];
        let image = NodeConfig::get_model(NodeModel::TalosLinux);
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
