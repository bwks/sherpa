use anyhow::{Context, Result, bail};

use crate::NodeExpanded;
use shared::data::{NodeModel, Ztp};
use shared::validators::check_ztp;

/// Check a node's `ztp.method` against the methods its model and kind take. A node that does not set it runs
/// with its image's setting, which is checked where the image is known.
///
/// # Errors
///
/// Fails when the node sets a method its model does not take.
pub fn check_node_ztp(node: &NodeExpanded) -> Result<()> {
    match node.ztp.method {
        Some(ztp) => check_ztp(node.model_kind, ztp)
            .with_context(|| format!("Manifest - node: '{}' sets `ztp.method`", node.name)),
        None => Ok(()),
    }
}

/// Check a node's own `ztp.config` is used with `ztp`, the ZTP setting it runs with. The methods
/// that deliver a config file take it in place of the generated one. Cloud-init, Ignition and the
/// HTTP config of `sonic_linux` are built from the manifest's other fields, and with ZTP off
/// nothing delivers it; the one exception is Vault, whose `ztp.config` is its own `config.hcl`, a
/// start setting.
///
/// # Errors
///
/// Fails when the node sets a `ztp.config` its ZTP setting would not use.
pub fn check_node_ztp_config(node: &NodeExpanded, ztp: Ztp) -> Result<()> {
    if node.ztp.config.is_none() {
        return Ok(());
    }
    let name = &node.name;
    match ztp {
        Ztp::Tftp | Ztp::Cdrom | Ztp::Disk | Ztp::Usb | Ztp::NoCloud | Ztp::Volume => Ok(()),
        Ztp::Disabled if node.model_kind.model() == NodeModel::HashicorpVault => Ok(()),
        Ztp::Disabled => bail!(
            "Manifest - node: '{name}' sets `ztp.config`, but its ZTP is disabled, so the config \
             would not be used"
        ),
        Ztp::CloudInit | Ztp::Ignition | Ztp::Http => bail!(
            "Manifest - node: '{name}' sets `ztp.config`, which ZTP method '{ztp}' does not use. \
             Only tftp, cdrom, disk, usb, nocloud and volume take a custom ZTP config"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeZtp;
    use shared::data::ModelKind;

    fn node(model: NodeModel, ztp_config: Option<&str>) -> NodeExpanded {
        NodeExpanded {
            name: "node1".to_owned(),
            model_kind: ModelKind::default_for(model),
            ztp: NodeZtp {
                method: None,
                config: ztp_config.map(str::to_owned),
            },
            ..NodeExpanded::default()
        }
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
            assert!(
                check_node_ztp_config(&node(model, Some("config")), ztp).is_ok(),
                "{model} with {ztp}"
            );
        }
        for (model, ztp) in [
            (NodeModel::UbuntuLinux, Ztp::CloudInit),
            (NodeModel::FlatcarLinux, Ztp::Ignition),
            (NodeModel::SonicLinux, Ztp::Http),
        ] {
            let error = check_node_ztp_config(&node(model, Some("config")), ztp)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!("which ZTP method '{ztp}' does not use")),
                "{model}: {error}"
            );
            assert!(
                check_node_ztp_config(&node(model, None), ztp).is_ok(),
                "{model}"
            );
        }
    }

    #[test]
    fn custom_config_006_a_config_with_ztp_off_is_refused_except_for_vault() {
        for model in NodeModel::to_vec() {
            let result = check_node_ztp_config(&node(model, Some("config")), Ztp::Disabled);
            if model == NodeModel::HashicorpVault {
                assert!(result.is_ok());
            } else {
                let error = result.unwrap_err().to_string();
                assert!(error.contains("its ZTP is disabled"), "{model}: {error}");
            }
        }
    }
}
