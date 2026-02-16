//! JWT token management for the Sherpa server.
//!
//! This module handles JWT secret generation, storage, token creation, and validation.
//! The JWT secret is automatically generated on first run and stored securely.

use anyhow::{Context, Result};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngCore;
use shared::auth::jwt::Claims;
use shared::konst::{JWT_SECRET_PATH, JWT_TOKEN_EXPIRY_SECONDS};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Load or generate a JWT secret.
///
/// This function:
/// 1. Checks if the JWT secret file exists at `/opt/sherpa/.secret/jwt.secret`
/// 2. If it exists, reads and returns it
/// 3. If it doesn't exist:
///    - Creates the `/opt/sherpa/.secret/` directory with 0700 permissions
///    - Generates a new 32-byte (256-bit) random secret
///    - Writes it to the file with 0600 permissions
///    - Returns the secret
///
/// # Returns
/// The JWT secret as a `Vec<u8>` (32 bytes)
///
/// # Errors
/// Returns an error if:
/// - Directory creation fails
/// - File I/O operations fail
/// - Permission setting fails
pub fn load_or_generate_secret() -> Result<Vec<u8>> {
    let secret_path = PathBuf::from(JWT_SECRET_PATH);

    // Check if secret file exists
    if secret_path.exists() {
        debug!("Loading existing JWT secret from {}", JWT_SECRET_PATH);
        let secret = fs::read(&secret_path).context("Failed to read JWT secret file")?;

        if secret.len() != 32 {
            warn!(
                "JWT secret file has unexpected length: {} bytes (expected 32). Regenerating.",
                secret.len()
            );
            return generate_and_save_secret(&secret_path);
        }

        info!("Successfully loaded JWT secret");
        Ok(secret)
    } else {
        info!(
            "JWT secret not found. Generating new secret at {}",
            JWT_SECRET_PATH
        );
        generate_and_save_secret(&secret_path)
    }
}

/// Generate a new secret and save it to the specified path.
fn generate_and_save_secret(secret_path: &Path) -> Result<Vec<u8>> {
    // Create parent directory with 0700 permissions
    let parent_dir = secret_path
        .parent()
        .context("Failed to get parent directory of JWT secret path")?;

    if !parent_dir.exists() {
        fs::create_dir_all(parent_dir).context("Failed to create .secret directory")?;

        // Set directory permissions to 0700 (owner read/write/execute only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::Permissions::from_mode(0o700);
            fs::set_permissions(parent_dir, perms)
                .context("Failed to set .secret directory permissions")?;
        }

        debug!("Created directory: {}", parent_dir.display());
    }

    // Generate 32-byte (256-bit) random secret
    let mut secret = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret);

    // Write secret to file
    fs::write(secret_path, &secret).context("Failed to write JWT secret file")?;

    // Set file permissions to 0600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(secret_path, perms)
            .context("Failed to set JWT secret file permissions")?;
    }

    info!("Successfully generated and saved new JWT secret");
    Ok(secret)
}

/// Create a JWT token for a user.
///
/// # Arguments
/// * `secret` - The JWT secret key (32 bytes)
/// * `username` - The username to encode in the token
/// * `is_admin` - Whether the user has admin privileges
/// * `expiry_seconds` - Token validity period in seconds (typically 7 days)
///
/// # Returns
/// A signed JWT token string
///
/// # Errors
/// Returns an error if token encoding fails
pub fn create_token(
    secret: &[u8],
    username: &str,
    is_admin: bool,
    expiry_seconds: i64,
) -> Result<String> {
    let claims = Claims::new(username.to_string(), is_admin, expiry_seconds);

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret),
    )
    .context("Failed to encode JWT token")?;

    debug!(
        "Created JWT token for user: {} (admin: {})",
        username, is_admin
    );
    Ok(token)
}

/// Validate a JWT token and extract claims.
///
/// # Arguments
/// * `secret` - The JWT secret key (32 bytes)
/// * `token` - The JWT token string to validate
///
/// # Returns
/// The decoded `Claims` if the token is valid and not expired
///
/// # Errors
/// Returns an error if:
/// - Token is malformed
/// - Token signature is invalid
/// - Token has expired
pub fn validate_token(secret: &[u8], token: &str) -> Result<Claims> {
    let mut validation = Validation::default();
    validation.validate_exp = true; // Explicitly enable expiration validation

    let token_data = decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation)
        .context("Failed to decode JWT token")?;

    debug!("Validated JWT token for user: {}", token_data.claims.sub);
    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_create_and_validate_token() {
        let secret = b"test_secret_32_bytes_long_enough";
        let username = "testuser";
        let is_admin = false;
        let expiry = 3600; // 1 hour

        // Create token
        let token =
            create_token(secret, username, is_admin, expiry).expect("Failed to create token");

        // Validate token
        let claims = validate_token(secret, &token).expect("Failed to validate token");

        assert_eq!(claims.sub, username);
        assert_eq!(claims.is_admin, is_admin);
        assert!(!claims.is_expired());
    }

    #[test]
    fn test_create_admin_token() {
        let secret = b"test_secret_32_bytes_long_enough";
        let token = create_token(secret, "admin", true, 3600).expect("Failed to create token");

        let claims = validate_token(secret, &token).expect("Failed to validate token");

        assert_eq!(claims.sub, "admin");
        assert_eq!(claims.is_admin, true);
    }

    #[test]
    fn test_validate_with_wrong_secret() {
        let secret1 = b"test_secret_32_bytes_long_enough";
        let secret2 = b"different_secret_32bytes_exactly";

        let token = create_token(secret1, "user", false, 3600).expect("Failed to create token");

        // Should fail with different secret
        let result = validate_token(secret2, &token);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_expired_token() {
        let secret = b"test_secret_32_bytes_long_enough";

        // Create token that's already expired (negative expiry)
        let now = chrono::Utc::now().timestamp();
        let expired_claims = Claims {
            sub: "user".to_string(),
            exp: now - 100, // Expired 100 seconds ago
            iat: now - 200,
            is_admin: false,
        };

        let token = encode(
            &Header::default(),
            &expired_claims,
            &EncodingKey::from_secret(secret),
        )
        .expect("Failed to encode token");

        // Validation should fail due to expiration
        let result = validate_token(secret, &token);
        assert!(
            result.is_err(),
            "Token validation should fail for expired token, got: {:?}",
            result
        );
    }

    #[test]
    #[ignore] // This test writes to filesystem
    fn test_generate_secret() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let secret_path = temp_dir.path().join("jwt.secret");

        let secret = generate_and_save_secret(&secret_path).expect("Failed to generate secret");

        // Verify secret length
        assert_eq!(secret.len(), 32);

        // Verify file exists
        assert!(secret_path.exists());

        // Verify file contents match
        let read_secret = fs::read(&secret_path).expect("Failed to read secret file");
        assert_eq!(secret, read_secret);

        // Verify file permissions on Unix
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&secret_path).expect("Failed to read metadata");
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
