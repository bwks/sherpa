//! JWT token management for the Sherpa server.
//!
//! This module handles JWT secret generation, storage, token creation, and validation.
//! The JWT secret is automatically generated on first run and stored securely.

use anyhow::{Context, Result, bail};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngCore;
use shared::auth::jwt::Claims;
use std::fs;
use std::path::Path;
use tracing::{debug, info};

use crate::tls::generator::write_file_with_permissions;

/// The length of the JWT secret, in bytes.
const SECRET_LENGTH: usize = 32;

/// Load the JWT secret from `secret_path`, or generate one there if the file does not exist.
///
/// A new secret is 32 random bytes, written 0600 in a 0700 directory.
///
/// # Errors
///
/// Fails when the file cannot be read or written, and when it exists with the wrong length. A
/// damaged secret is not replaced: replacing it would silently end every session, so the server
/// refuses to start and the error names the file. Deleting the file generates a new secret, which
/// logs every user out.
#[tracing::instrument(skip_all, fields(path = %secret_path.display()))]
pub fn load_or_generate_secret(secret_path: &Path) -> Result<Vec<u8>> {
    if !secret_path.exists() {
        info!("JWT secret not found. Generating a new secret");
        return generate_and_save_secret(secret_path);
    }
    debug!("Loading existing JWT secret");
    let secret = fs::read(secret_path)
        .with_context(|| format!("Failed to read the JWT secret {}", secret_path.display()))?;
    if secret.len() != SECRET_LENGTH {
        bail!(
            "The JWT secret {} is {} bytes long, not {SECRET_LENGTH}. Restore it from a backup, or \
             delete it to generate a new one, which logs every user out",
            secret_path.display(),
            secret.len()
        );
    }
    info!("Successfully loaded JWT secret");
    Ok(secret)
}

/// Generate a new secret and save it to the specified path.
fn generate_and_save_secret(secret_path: &Path) -> Result<Vec<u8>> {
    // Generate 32-byte (256-bit) random secret
    let mut secret = vec![0u8; SECRET_LENGTH];
    rand::thread_rng().fill_bytes(&mut secret);

    // Created 0600 in a 0700 directory, so the secret is never readable by others.
    write_file_with_permissions(secret_path, &secret, 0o600)
        .context("Failed to write JWT secret file")?;

    info!("Successfully generated and saved new JWT secret");
    Ok(secret)
}

/// Sign a token carrying `claims`.
///
/// # Errors
/// Returns an error if token encoding fails
pub fn create_token(secret: &[u8], claims: &Claims) -> Result<String> {
    let token = encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(secret),
    )
    .context("Failed to encode JWT token")?;
    debug!(
        "Created JWT token for user: {} (admin: {})",
        claims.sub, claims.is_admin
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

/// Validate a token's signature and claims while allowing an expired `exp` value.
///
/// This is used only to recover safe diagnostic metadata from an expired token.
#[tracing::instrument(level = "debug", skip(secret, token))]
pub(crate) fn validate_expired_token_for_diagnostics(secret: &[u8], token: &str) -> Result<Claims> {
    let mut validation = Validation::default();
    validation.validate_exp = false;

    let token_data = decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation)
        .context("Failed to validate expired JWT token for diagnostics")?;

    Ok(token_data.claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    /// Claims for `username`, valid for `expiry` seconds from now.
    fn claims(username: &str, is_admin: bool, expiry: i64) -> Claims {
        let now = jiff::Timestamp::now().as_second();
        Claims::new(
            username.to_owned(),
            "session".to_owned(),
            is_admin,
            now,
            now + expiry,
        )
    }

    #[test]
    fn test_create_and_validate_token() {
        let secret = b"test_secret_32_bytes_long_enough";
        let username = "testuser";
        let is_admin = false;
        let expiry = 3600; // 1 hour

        // Create token
        let token = create_token(secret, &claims(username, is_admin, expiry))
            .expect("Failed to create token");

        // Validate token
        let claims = validate_token(secret, &token).expect("Failed to validate token");

        assert_eq!(claims.sub, username);
        assert_eq!(claims.is_admin, is_admin);
    }

    #[test]
    fn test_create_admin_token() {
        let secret = b"test_secret_32_bytes_long_enough";
        let token =
            create_token(secret, &claims("admin", true, 3600)).expect("Failed to create token");

        let claims = validate_token(secret, &token).expect("Failed to validate token");

        assert_eq!(claims.sub, "admin");
        assert!(claims.is_admin);
    }

    #[test]
    fn test_validate_with_wrong_secret() {
        let secret1 = b"test_secret_32_bytes_long_enough";
        let secret2 = b"different_secret_32bytes_exactly";

        let token =
            create_token(secret1, &claims("user", false, 3600)).expect("Failed to create token");

        // Should fail with different secret
        let result = validate_token(secret2, &token);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_expired_token() {
        let secret = b"test_secret_32_bytes_long_enough";

        // Create token that's already expired (negative expiry)
        let now = jiff::Timestamp::now().as_second();
        let expired_claims = Claims {
            sub: "user".to_string(),
            sid: "session".to_string(),
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
            "Token validation should fail for expired token, got: {result:?}"
        );
    }

    #[test]
    fn test_validate_malformed_token() {
        let secret = b"test_secret_32_bytes_long_enough";
        let result = validate_token(secret, "not.a.valid.jwt");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_empty_token() {
        let secret = b"test_secret_32_bytes_long_enough";
        let result = validate_token(secret, "");
        assert!(result.is_err());
    }

    #[test]
    fn test_token_expiry_matches_requested() {
        let secret = b"test_secret_32_bytes_long_enough";
        let expiry = 604_800; // 7 days
        let token =
            create_token(secret, &claims("user", false, expiry)).expect("Failed to create token");
        let claims = validate_token(secret, &token).expect("Failed to validate token");
        assert_eq!(claims.exp - claims.iat, expiry);
    }

    #[test]
    fn a_secret_of_the_wrong_length_refuses_startup_and_is_kept() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let secret_path = temp_dir.path().join("jwt.secret");
        fs::write(&secret_path, b"too short").expect("write secret");

        let error = load_or_generate_secret(&secret_path).expect_err("short secret accepted");

        let message = format!("{error:#}");
        assert!(
            message.contains(&secret_path.display().to_string()),
            "the error does not name the file: {message}"
        );
        assert!(message.contains("9 bytes"), "{message}");
        assert_eq!(
            fs::read(&secret_path).expect("read secret"),
            b"too short",
            "the damaged secret was replaced"
        );
    }

    #[test]
    fn a_secret_of_the_right_length_is_loaded() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let secret_path = temp_dir.path().join("jwt.secret");
        fs::write(&secret_path, [7u8; 32]).expect("write secret");

        assert_eq!(
            load_or_generate_secret(&secret_path).expect("load"),
            vec![7u8; 32]
        );
    }

    #[test]
    #[ignore = "writes the JWT secret file to the server's filesystem"]
    fn test_generate_secret() {
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

        let mode = fs::metadata(&secret_path)
            .expect("Failed to read metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
