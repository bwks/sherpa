use anyhow::{Result, anyhow, bail};
use tracing::instrument;

use shared::data::NodeConfig;
use shared::util;

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
        None => Ok(node_image.data_interface_count),
    }
}

#[instrument(skip(node_image), fields(node_name = %node_name, node_model = %node_image.model), level = "debug")]
pub fn validate_data_interface_count_override(
    node_name: &str,
    requested_count: u8,
    node_image: &NodeConfig,
) -> Result<()> {
    let requested_max_idx = node_image
        .reserved_interface_count
        .checked_add(requested_count)
        .ok_or_else(|| {
            anyhow!(
                "Node '{}' (model: {}) requested data_interface_count {} but reserved interface count {} causes interface index overflow",
                node_name,
                node_image.model,
                requested_count,
                node_image.reserved_interface_count
            )
        })?;

    if util::interface_from_idx(&node_image.model, requested_max_idx).is_err() {
        let max_supported_idx = util::node_model_interfaces(&node_image.model)
            .iter()
            .filter_map(|interface| util::interface_to_idx(&node_image.model, interface).ok())
            .max()
            .unwrap_or(0);
        let max_allowed_count =
            max_supported_idx.saturating_sub(node_image.reserved_interface_count);

        bail!(
            "Node '{}' (model: {}) requested data_interface_count {} but only 0..={} data interfaces are supported (reserved: {}, max interface index: {})",
            node_name,
            node_image.model,
            requested_count,
            max_allowed_count,
            node_image.reserved_interface_count,
            max_supported_idx
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::data::NodeConfig;

    #[test]
    fn test_effective_data_interface_count_uses_default_without_override() {
        let node_image = NodeConfig::ubuntu_linux();
        let count = effective_data_interface_count("dev01", None, &node_image).unwrap();
        assert_eq!(count, node_image.data_interface_count);
    }

    #[test]
    fn test_effective_data_interface_count_uses_manifest_override() {
        let node_image = NodeConfig::ubuntu_linux();
        let count = effective_data_interface_count("dev01", Some(4), &node_image).unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn test_validate_data_interface_count_override_rejects_unsupported_count() {
        let node_image = NodeConfig::ubuntu_linux();
        let result = effective_data_interface_count("dev01", Some(53), &node_image);
        assert!(result.is_err());
        let error = result.unwrap_err().to_string();
        assert!(error.contains("dev01"));
        assert!(error.contains("ubuntu_linux"));
        assert!(error.contains("data_interface_count 53"));
    }

    #[test]
    fn test_validate_data_interface_count_override_rejects_index_overflow() {
        let mut node_image = NodeConfig::ubuntu_linux();
        node_image.reserved_interface_count = 250;
        let result = effective_data_interface_count("dev01", Some(10), &node_image);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("overflow"));
    }
}
