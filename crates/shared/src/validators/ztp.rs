use std::iter;

use anyhow::{Result, bail};

use crate::data::{ModelKind, NodeConfig, Ztp};

/// The ZTP setting a node runs with: the manifest's when the node sets one, or else its
/// image's. Every reader of the setting goes through this.
#[must_use]
pub fn effective_ztp(manifest_ztp: Option<Ztp>, node_image: &NodeConfig) -> Ztp {
    manifest_ztp.unwrap_or(node_image.ztp())
}

/// Check a ZTP setting against the methods its model and kind take. Used for an image's
/// setting and for a manifest node's override alike.
///
/// # Errors
///
/// Fails when the setting is a method the pair does not take. The error names the model, the
/// setting and the methods the pair allows.
pub fn check_ztp(model_kind: ModelKind, ztp: Ztp) -> Result<()> {
    if ztp == Ztp::Disabled || model_kind.ztp_methods().contains(&ztp) {
        return Ok(());
    }
    let model = model_kind.model();
    let allowed = iter::once(Ztp::Disabled)
        .chain(model_kind.ztp_methods().iter().copied())
        .map(|allowed| allowed.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    bail!("ZTP setting '{ztp}' is not supported by model {model}, which allows: {allowed}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::NodeModel;

    #[test]
    fn ztp_setting_003_disabled_is_taken_for_every_model_and_kind() {
        for model_kind in ModelKind::all() {
            assert!(
                check_ztp(model_kind, Ztp::Disabled).is_ok(),
                "{model_kind:?}"
            );
        }
    }

    #[test]
    fn ztp_setting_004_a_method_the_model_does_not_take_is_refused_naming_what_it_takes() {
        let error = check_ztp(
            ModelKind::default_for(NodeModel::JuniperEvolved),
            Ztp::Cdrom,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("juniper_evolved"), "{error}");
        assert!(error.contains("'cdrom'"), "{error}");
        assert!(error.ends_with("allows: disabled, tftp, usb"), "{error}");

        let error = check_ztp(ModelKind::default_for(NodeModel::MongoDb), Ztp::Volume)
            .unwrap_err()
            .to_string();
        assert!(error.contains("mongo_db"), "{error}");
        assert!(error.ends_with("allows: disabled"), "{error}");
    }
}
