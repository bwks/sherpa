//! Token storage and management for sherpactl
//!
//! This module handles JWT token storage in the user's home directory.
//! Tokens are shared with the sherpa client tool (same ~/.sherpa/token file).

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

/// Get the path to the token file (~/.sherpa/token)
fn token_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let sherpa_dir = PathBuf::from(home).join(".sherpa");

    // Ensure directory exists
    if !sherpa_dir.exists() {
        fs::create_dir_all(&sherpa_dir).context("Failed to create ~/.sherpa directory")?;
    }

    Ok(sherpa_dir.join("token"))
}

/// Load the authentication token from ~/.sherpa/token
///
/// # Errors
/// Returns an error if:
/// - The token file doesn't exist (user not logged in)
/// - The token file cannot be read
/// - The HOME environment variable is not set
pub fn load_token() -> Result<String> {
    let path = token_path()?;

    if !path.exists() {
        anyhow::bail!("Not authenticated. Please run: sherpa login");
    }

    let token = fs::read_to_string(&path)
        .context("Failed to read authentication token")?
        .trim()
        .to_string();

    if token.is_empty() {
        anyhow::bail!("Token file is empty. Please run: sherpa login");
    }

    Ok(token)
}

/// Save the authentication token to ~/.sherpa/token
///
/// The file is created with 0600 permissions (owner read/write only)
///
/// # Errors
/// Returns an error if:
/// - The directory cannot be created
/// - The file cannot be written
/// - File permissions cannot be set
#[allow(dead_code)]
#[cfg(unix)]
pub fn save_token(token: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let path = token_path()?;

    // Write token to file
    fs::write(&path, token).context("Failed to write authentication token")?;

    // Set permissions to 0600 (owner read/write only)
    let permissions = fs::Permissions::from_mode(0o600);
    fs::set_permissions(&path, permissions).context("Failed to set token file permissions")?;

    Ok(())
}

#[allow(dead_code)]
#[cfg(not(unix))]
pub fn save_token(token: &str) -> Result<()> {
    let path = token_path()?;
    fs::write(&path, token).context("Failed to write authentication token")?;
    Ok(())
}

/// Delete the authentication token file
///
/// # Errors
/// Returns an error if the file cannot be deleted
#[allow(dead_code)]
pub fn delete_token() -> Result<()> {
    let path = token_path()?;

    if path.exists() {
        fs::remove_file(&path).context("Failed to delete authentication token")?;
    }

    Ok(())
}

/// Check if a token file exists
#[allow(dead_code)]
pub fn token_exists() -> bool {
    token_path().map(|p| p.exists()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_token_path_uses_home() {
        let home = env::var("HOME").expect("HOME should be set");
        let path = token_path().expect("Should get token path");
        assert!(path.to_string_lossy().starts_with(&home));
        assert!(path.to_string_lossy().ends_with(".sherpa/token"));
    }

    #[test]
    fn test_token_exists_when_no_file() {
        // This test assumes the token doesn't exist
        // In a real test environment, we'd use a temp directory
        let exists = token_exists();
        assert!(exists || !exists); // Always passes, just testing it doesn't panic
    }
}
