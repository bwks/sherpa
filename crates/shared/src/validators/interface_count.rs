use anyhow::Result;
use tracing::instrument;

use crate::SharedError;
use crate::data::NodeConfig;
use crate::util;

/// The number of data interfaces a node gets: the manifest's count when it sets one, checked
/// against the model, or else the image's default.
///
/// # Errors
///
/// Fails when the override is more interfaces than the model has.
#[instrument(skip(node_image), fields(node_name = %node_name, node_model = %node_image.model), level = "debug")]
pub fn effective_data_interface_count(
    node_name: &str,
    manifest_count: Option<u8>,
    node_image: &NodeConfig,
) -> Result<u8> {
    match manifest_count {
        Some(count) => {
            validate_data_interface_count_override(node_name, count, node_image)?;
            Ok(count)
        }
        None => Ok(node_image.interfaces.data_count),
    }
}

/// Check a manifest's data interface count for a node against what its model has.
///
/// # Errors
///
/// Fails when the override is more interfaces than the model has.
#[instrument(skip(node_image), fields(node_name = %node_name, node_model = %node_image.model), level = "debug")]
pub fn validate_data_interface_count_override(
    node_name: &str,
    requested_count: u8,
    node_image: &NodeConfig,
) -> Result<()> {
    let requested_max_idx = node_image
        .interfaces
        .reserved_count
        .checked_add(requested_count)
        .ok_or_else(|| SharedError::DataInterfaceIndexOverflow {
            node: node_name.to_owned(),
            model: node_image.model,
            requested: requested_count,
            reserved: node_image.interfaces.reserved_count,
        })?;

    let model_kind = node_image.model_kind()?;
    if util::interface_from_idx(model_kind, requested_max_idx).is_err() {
        let max_supported_idx = util::node_model_interfaces(model_kind)
            .iter()
            .filter_map(|interface| util::interface_to_idx(model_kind, interface).ok())
            .max()
            .unwrap_or(0);
        let max_allowed_count =
            max_supported_idx.saturating_sub(node_image.interfaces.reserved_count);

        return Err(SharedError::DataInterfaceCountUnsupported {
            node: node_name.to_owned(),
            model: node_image.model,
            requested: requested_count,
            max_allowed: max_allowed_count,
            reserved: node_image.interfaces.reserved_count,
            max_index: max_supported_idx,
        }
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{ModelKind, NodeConfig, NodeModel};

    #[test]
    fn node_settings_010_a_data_count_past_the_models_interfaces_is_refused_with_the_most() {
        let node_image = NodeConfig::ubuntu_linux();
        let error = effective_data_interface_count("dev01", Some(53), &node_image).unwrap_err();
        let Some(SharedError::DataInterfaceCountUnsupported {
            node,
            model,
            requested,
            max_allowed,
            ..
        }) = error.downcast_ref::<SharedError>()
        else {
            panic!("not an unsupported data interface count: {error:#}");
        };
        assert_eq!(node, "dev01");
        assert_eq!(*model, NodeModel::UbuntuLinux);
        assert_eq!(*requested, 53);
        assert_eq!(*max_allowed, 52);
    }

    #[test]
    fn node_settings_011_a_data_count_past_the_last_index_with_reserved_ones_is_an_overflow() {
        let mut node_image = NodeConfig::ubuntu_linux();
        node_image.interfaces.reserved_count = 250;
        let error = effective_data_interface_count("dev01", Some(10), &node_image).unwrap_err();
        assert!(matches!(
            error.downcast_ref::<SharedError>(),
            Some(SharedError::DataInterfaceIndexOverflow { .. })
        ));
    }

    #[test]
    fn node_settings_012_every_models_built_in_data_count_fits_the_interfaces_the_model_has() {
        let mut overruns = Vec::new();
        for model_kind in ModelKind::all() {
            let node_image = model_kind.node_config();
            if let Err(err) = validate_data_interface_count_override(
                "default",
                node_image.interfaces.data_count,
                &node_image,
            ) {
                overruns.push(format!("{model_kind}: {err}"));
            }
        }
        assert!(overruns.is_empty(), "{}", overruns.join("\n"));
    }
}
