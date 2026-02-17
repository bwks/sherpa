/// SSH key validation utilities
use anyhow::{Result, bail};

/// Validates an SSH public key format
///
/// Valid SSH keys have the format: `algorithm key_data [optional_comment]`
/// where algorithm is one of: ssh-rsa, ssh-ed25519, ssh-dss, ecdsa-sha2-*
///
/// # Arguments
/// * `key` - The SSH public key string to validate
///
/// # Returns
/// * `Ok(())` if the key is valid
/// * `Err` with a descriptive message if invalid
///
/// # Example
/// ```
/// use shared::auth::ssh::validate_ssh_key;
///
/// let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJb... user@host";
/// assert!(validate_ssh_key(key).is_ok());
/// ```
pub fn validate_ssh_key(key: &str) -> Result<()> {
    // Trim whitespace
    let key = key.trim();

    if key.is_empty() {
        bail!("SSH key cannot be empty");
    }

    // Split into parts
    let parts: Vec<&str> = key.split_whitespace().collect();

    if parts.len() < 2 {
        bail!("SSH key must contain at least algorithm and key data");
    }

    let algorithm = parts[0];
    let key_data = parts[1];

    // Validate algorithm
    let valid_algorithms = [
        "ssh-rsa",
        "ssh-ed25519",
        "ssh-dss",
        "ecdsa-sha2-nistp256",
        "ecdsa-sha2-nistp384",
        "ecdsa-sha2-nistp521",
    ];

    if !valid_algorithms.contains(&algorithm) {
        bail!(
            "Invalid SSH key algorithm '{}'. Supported algorithms: {}",
            algorithm,
            valid_algorithms.join(", ")
        );
    }

    // Validate key data is base64
    // SSH keys use base64 encoding, which only contains: A-Z, a-z, 0-9, +, /, and = for padding
    if !key_data
        .chars()
        .all(|c| c.is_alphanumeric() || c == '+' || c == '/' || c == '=')
    {
        bail!("SSH key data must be valid base64");
    }

    // Key data should be reasonably long (at least 32 characters for the smallest keys)
    if key_data.len() < 32 {
        bail!("SSH key data is too short to be valid");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_ssh_ed25519_key() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh user@host";
        assert!(validate_ssh_key(key).is_ok());
    }

    #[test]
    fn test_valid_ssh_rsa_key() {
        let key = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABgQC8Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0Z0";
        assert!(validate_ssh_key(key).is_ok());
    }

    #[test]
    fn test_valid_ecdsa_key() {
        let key = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBEmKSENjQEezOmxkZMy7opKgwFB9nkt5YRrYMjNuG5N87uRgg6CLrbo5wAdT/y6v0mKV0U2w0WZ2YB/++Tpockg=";
        assert!(validate_ssh_key(key).is_ok());
    }

    #[test]
    fn test_key_without_comment() {
        let key =
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";
        assert!(validate_ssh_key(key).is_ok());
    }

    #[test]
    fn test_empty_key() {
        assert!(validate_ssh_key("").is_err());
        assert!(validate_ssh_key("   ").is_err());
    }

    #[test]
    fn test_invalid_algorithm() {
        let key =
            "invalid-algo AAAAC3NzaC1lZDI1NTE5AAAAIJbRB5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5JhR5Jh";
        assert!(validate_ssh_key(key).is_err());
    }

    #[test]
    fn test_missing_key_data() {
        assert!(validate_ssh_key("ssh-ed25519").is_err());
    }

    #[test]
    fn test_invalid_base64() {
        let key = "ssh-ed25519 Invalid!@#$%^&*()Characters user@host";
        assert!(validate_ssh_key(key).is_err());
    }

    #[test]
    fn test_key_data_too_short() {
        let key = "ssh-ed25519 ABC123";
        assert!(validate_ssh_key(key).is_err());
    }
}
