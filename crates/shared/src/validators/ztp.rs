use std::iter;

use anyhow::Result;

use crate::SharedError;
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
/// `SharedError::UnsupportedZtp` when the setting is a method the pair does not take, carrying
/// the model, the setting and the methods the pair allows.
pub fn check_ztp(model_kind: ModelKind, ztp: Ztp) -> Result<()> {
    if ztp == Ztp::Disabled || model_kind.ztp_methods().contains(&ztp) {
        return Ok(());
    }
    Err(SharedError::UnsupportedZtp {
        ztp,
        model: model_kind.model(),
        allowed: iter::once(Ztp::Disabled)
            .chain(model_kind.ztp_methods().iter().copied())
            .collect(),
    }
    .into())
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
    fn ztp_setting_004_a_method_the_model_does_not_take_is_refused_with_what_it_takes() {
        let error = check_ztp(
            ModelKind::default_for(NodeModel::JuniperEvolved),
            Ztp::Cdrom,
        )
        .unwrap_err();
        assert_eq!(
            error.downcast_ref::<SharedError>(),
            Some(&SharedError::UnsupportedZtp {
                ztp: Ztp::Cdrom,
                model: NodeModel::JuniperEvolved,
                allowed: vec![Ztp::Disabled, Ztp::Tftp, Ztp::Usb],
            })
        );

        let error = check_ztp(ModelKind::default_for(NodeModel::MongoDb), Ztp::Volume).unwrap_err();
        assert_eq!(
            error.downcast_ref::<SharedError>(),
            Some(&SharedError::UnsupportedZtp {
                ztp: Ztp::Volume,
                model: NodeModel::MongoDb,
                allowed: vec![Ztp::Disabled],
            })
        );
    }
}
