use anyhow::{Result, bail};

use crate::services::error::ServiceError;

/// The longest image version.
const IMAGE_VERSION_MAX_LEN: usize = 64;

/// Validate node image form data before database update
pub fn validate_node_image_update(
    cpu_count: u8,
    memory: u16,
    interface_mtu: u16,
    version: &str,
    interface_prefix: &str,
) -> Result<()> {
    // Validate numeric ranges
    validate_cpu_count(cpu_count)?;
    validate_memory(memory)?;
    validate_interface_mtu(interface_mtu)?;

    // Validate string fields
    validate_image_version(version)?;
    validate_interface_prefix(interface_prefix)?;

    Ok(())
}

fn validate_cpu_count(count: u8) -> Result<()> {
    if count < 1 {
        bail!("CPU count must be at least 1");
    }
    Ok(())
}

fn validate_memory(mb: u16) -> Result<()> {
    if mb < 64 {
        bail!("Memory must be at least 64 MB");
    }
    Ok(())
}

fn validate_interface_mtu(mtu: u16) -> Result<()> {
    if !(576..=9600).contains(&mtu) {
        bail!("Interface MTU must be between 576 and 9600");
    }
    Ok(())
}

/// Check an image version, which names the image's folder and its container tag.
///
/// A version is 1 to 64 letters, digits, `.`, `_`, `+` or `-`, starting with a letter or digit,
/// so it can never be a path such as `..` or `a/b`.
///
/// # Errors
///
/// Fails with `ServiceError::Invalid` when `version` breaks the rule.
pub fn validate_image_version(version: &str) -> Result<()> {
    let starts_well = version
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric());
    let plain = version
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'));
    if starts_well && plain && version.len() <= IMAGE_VERSION_MAX_LEN {
        return Ok(());
    }
    Err(ServiceError::Invalid(format!(
        "Image version '{version}' is not allowed: use 1 to {IMAGE_VERSION_MAX_LEN} letters, \
         digits, '.', '_', '+' or '-', starting with a letter or digit"
    ))
    .into())
}

fn validate_interface_prefix(prefix: &str) -> Result<()> {
    if prefix.trim().is_empty() {
        bail!("Interface prefix cannot be empty");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_cpu_count() {
        assert!(validate_cpu_count(1).is_ok());
        assert!(validate_cpu_count(255).is_ok());
        assert!(validate_cpu_count(0).is_err());
    }

    #[test]
    fn test_validate_memory() {
        assert!(validate_memory(64).is_ok());
        assert!(validate_memory(1024).is_ok());
        assert!(validate_memory(63).is_err());
    }

    #[test]
    fn test_validate_interface_mtu() {
        assert!(validate_interface_mtu(576).is_ok());
        assert!(validate_interface_mtu(1500).is_ok());
        assert!(validate_interface_mtu(9600).is_ok());
        assert!(validate_interface_mtu(575).is_err());
        assert!(validate_interface_mtu(9601).is_err());
    }

    #[test]
    fn real_image_versions_are_accepted() {
        let longest = "1".repeat(64);
        for version in [
            "1.0",
            "24.4",
            "17",
            "9-23-1",
            "latest",
            "26.2R1.7-EVO",
            "v1.9.0+k3s1",
            "abc_1",
            longest.as_str(),
        ] {
            assert!(validate_image_version(version).is_ok(), "{version}");
        }
    }

    #[test]
    fn versions_that_are_paths_or_odd_names_are_refused() {
        let too_long = "1".repeat(65);
        for version in [
            "",
            " ",
            "..",
            ".",
            "../images",
            "a/b",
            ".hidden",
            "-x",
            "_x",
            "1 0",
            "1.0\n",
            too_long.as_str(),
        ] {
            assert!(validate_image_version(version).is_err(), "{version:?}");
        }
    }

    #[test]
    fn test_validate_interface_prefix() {
        assert!(validate_interface_prefix("eth").is_ok());
        assert!(validate_interface_prefix("ge-0/0/").is_ok());
        assert!(validate_interface_prefix("").is_err());
        assert!(validate_interface_prefix("   ").is_err());
    }

    #[test]
    fn test_validate_node_image_update_all_valid() {
        let result = validate_node_image_update(2, 1024, 1500, "1.0.0", "eth");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_node_image_update_cpu_zero() {
        let result = validate_node_image_update(0, 1024, 1500, "1.0.0", "eth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("CPU count"));
    }

    #[test]
    fn test_validate_node_image_update_memory_too_low() {
        let result = validate_node_image_update(1, 32, 1500, "1.0.0", "eth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Memory"));
    }

    #[test]
    fn test_validate_node_image_update_mtu_out_of_range() {
        let result = validate_node_image_update(1, 1024, 100, "1.0.0", "eth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("MTU"));
    }

    #[test]
    fn test_validate_node_image_update_empty_version() {
        let result = validate_node_image_update(1, 1024, 1500, "", "eth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Image version"));
    }

    #[test]
    fn test_validate_node_image_update_empty_prefix() {
        let result = validate_node_image_update(1, 1024, 1500, "1.0.0", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Interface prefix"));
    }

    #[test]
    fn test_validate_node_image_update_boundary_values() {
        // All fields at minimum valid values
        let result = validate_node_image_update(1, 64, 576, "abcde", "e");
        assert!(result.is_ok());
    }
}
