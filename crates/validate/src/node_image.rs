use anyhow::{Result, bail};

/// Validate node image form data before database update
pub fn validate_node_image_update(
    cpu_count: u8,
    memory: u16,
    data_interface_count: u8,
    interface_mtu: u16,
    version: &str,
    interface_prefix: &str,
) -> Result<()> {
    // Validate numeric ranges
    validate_cpu_count(cpu_count)?;
    validate_memory(memory)?;
    validate_interface_count(data_interface_count)?;
    validate_interface_mtu(interface_mtu)?;

    // Validate string fields
    validate_version(version)?;
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

fn validate_interface_count(count: u8) -> Result<()> {
    if count < 1 {
        bail!("Data interface count must be at least 1");
    }
    Ok(())
}

fn validate_interface_mtu(mtu: u16) -> Result<()> {
    if !(576..=9600).contains(&mtu) {
        bail!("Interface MTU must be between 576 and 9600");
    }
    Ok(())
}

fn validate_version(version: &str) -> Result<()> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        bail!("Version cannot be empty");
    }
    if trimmed.len() <= 4 {
        bail!("Version must be more than 4 characters");
    }
    Ok(())
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
    fn test_validate_interface_count() {
        assert!(validate_interface_count(1).is_ok());
        assert!(validate_interface_count(255).is_ok());
        assert!(validate_interface_count(0).is_err());
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
    fn test_validate_version() {
        assert!(validate_version("1.0.0").is_ok());
        assert!(validate_version("latest").is_ok());
        assert!(validate_version("v2024").is_ok());
        assert!(validate_version("").is_err());
        assert!(validate_version("   ").is_err());
        assert!(validate_version("1.0").is_err()); // 3 chars
        assert!(validate_version("abcd").is_err()); // 4 chars
        assert!(validate_version("abcde").is_ok()); // 5 chars (> 4)
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
        let result = validate_node_image_update(2, 1024, 4, 1500, "1.0.0", "eth");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_node_image_update_cpu_zero() {
        let result = validate_node_image_update(0, 1024, 4, 1500, "1.0.0", "eth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("CPU count"));
    }

    #[test]
    fn test_validate_node_image_update_memory_too_low() {
        let result = validate_node_image_update(1, 32, 4, 1500, "1.0.0", "eth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Memory"));
    }

    #[test]
    fn test_validate_node_image_update_interface_count_zero() {
        let result = validate_node_image_update(1, 1024, 0, 1500, "1.0.0", "eth");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Data interface count")
        );
    }

    #[test]
    fn test_validate_node_image_update_mtu_out_of_range() {
        let result = validate_node_image_update(1, 1024, 4, 100, "1.0.0", "eth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("MTU"));
    }

    #[test]
    fn test_validate_node_image_update_empty_version() {
        let result = validate_node_image_update(1, 1024, 4, 1500, "", "eth");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Version"));
    }

    #[test]
    fn test_validate_node_image_update_empty_prefix() {
        let result = validate_node_image_update(1, 1024, 4, 1500, "1.0.0", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Interface prefix"));
    }

    #[test]
    fn test_validate_node_image_update_boundary_values() {
        // All fields at minimum valid values
        let result = validate_node_image_update(1, 64, 1, 576, "abcde", "e");
        assert!(result.is_ok());
    }
}
