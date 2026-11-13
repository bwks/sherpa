use std::iter;

use anyhow::{Result, bail};

use crate::data::{NodeConfig, NodeModel, Ztp};

/// The ZTP setting a node runs with: the manifest's when the node sets one, or else its
/// image's. Every reader of the setting goes through this.
#[must_use]
pub fn effective_ztp(manifest_ztp: Option<Ztp>, node_image: &NodeConfig) -> Ztp {
    manifest_ztp.unwrap_or(node_image.ztp.method)
}

/// Check a ZTP setting against the methods its model takes. Used for an image's setting and
/// for a manifest node's override alike.
///
/// # Errors
///
/// Fails when the setting is a method the model does not take. The error names the model, the
/// setting and the methods the model allows.
pub fn check_ztp(model: NodeModel, ztp: Ztp) -> Result<()> {
    if ztp == Ztp::Disabled || model.ztp_methods().contains(&ztp) {
        return Ok(());
    }
    let allowed = iter::once(Ztp::Disabled)
        .chain(model.ztp_methods().iter().copied())
        .map(|allowed| allowed.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    bail!("ZTP setting '{ztp}' is not supported by model {model}, which allows: {allowed}")
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn test_disabled_passes_for_every_model() {
        for model in NodeModel::iter() {
            assert!(check_ztp(model, Ztp::Disabled).is_ok(), "{model}");
        }
    }

    #[test]
    fn test_a_method_passes_exactly_when_the_model_lists_it() {
        for model in NodeModel::iter() {
            for ztp in Ztp::iter().filter(|ztp| *ztp != Ztp::Disabled) {
                assert_eq!(
                    check_ztp(model, ztp).is_ok(),
                    model.ztp_methods().contains(&ztp),
                    "{model} with {ztp}"
                );
            }
        }
    }

    #[test]
    fn test_the_error_names_the_model_the_value_and_the_allowed_methods() {
        let error = check_ztp(NodeModel::JuniperVsrxv3, Ztp::Usb)
            .unwrap_err()
            .to_string();
        assert!(error.contains("juniper_vsrxv3"), "{error}");
        assert!(error.contains("'usb'"), "{error}");
        assert!(error.contains("disabled, tftp, cdrom"), "{error}");
    }

    #[test]
    fn test_the_error_for_a_model_without_methods_allows_only_disabled() {
        let error = check_ztp(NodeModel::MongoDb, Ztp::Volume)
            .unwrap_err()
            .to_string();
        assert!(error.contains("mongo_db"), "{error}");
        assert!(error.ends_with("allows: disabled"), "{error}");
    }

    #[test]
    fn test_effective_ztp_is_the_images_when_the_manifest_sets_none() {
        let image = NodeConfig::get_model(NodeModel::FrrLinux);
        assert_eq!(effective_ztp(None, &image), Ztp::Volume);
    }

    #[test]
    fn test_effective_ztp_prefers_the_manifests() {
        let image = NodeConfig::get_model(NodeModel::FrrLinux);
        assert_eq!(effective_ztp(Some(Ztp::Disabled), &image), Ztp::Disabled);
        let image = NodeConfig::get_model(NodeModel::JuniperVsrxv3);
        assert_eq!(effective_ztp(Some(Ztp::Tftp), &image), Ztp::Tftp);
    }
}
