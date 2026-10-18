use anyhow::{Result, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand::rngs::OsRng;

use crate::validators::validate_password_strength;

/// Hashes a password using Argon2id algorithm.
///
/// First validates password strength, then hashes with Argon2id using default parameters
/// and a random salt. Returns the password hash in PHC string format.
///
/// # Errors
///
/// Returns an error if password validation fails or hashing fails.
pub fn hash_password(password: &str) -> Result<String> {
    // Validate password strength first
    validate_password_strength(password)?;

    // Generate a random salt
    let salt = SaltString::generate(&mut OsRng);

    // Hash the password with Argon2id (default parameters)
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("Failed to hash password: {}", e))?
        .to_string();

    Ok(password_hash)
}

/// Verifies a password against its hash.
///
/// Uses constant-time comparison to prevent timing attacks.
///
/// # Errors
///
/// Returns an error if the hash format is invalid.
/// Returns Ok(false) if the password does not match.
/// Returns Ok(true) if the password matches.
pub fn verify_password(password: &str, hash: &str) -> Result<bool> {
    let parsed_hash = PasswordHash::new(hash)
        .map_err(|error| anyhow!("Invalid password hash format: {error}"))?;

    let argon2 = Argon2::default();

    // verify_password returns Result<(), Error>, we convert to bool
    match argon2.verify_password(password.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_password_valid() {
        let password = "SecurePass123!";
        let hash = hash_password(password).expect("Should hash valid password");

        // Verify the hash starts with $argon2id$ (PHC format)
        assert!(hash.starts_with("$argon2id$"));
    }

    #[test]
    fn test_hash_password_invalid() {
        let password = "weak";
        assert!(hash_password(password).is_err());
    }

    #[test]
    fn test_hash_password_different_each_time() {
        let password = "SecurePass123!";
        let hash1 = hash_password(password).expect("Should hash password");
        let hash2 = hash_password(password).expect("Should hash password");

        // Same password should produce different hashes due to random salt
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_verify_password_correct() {
        let password = "SecurePass123!";
        let hash = hash_password(password).expect("Should hash password");

        let result = verify_password(password, &hash).expect("Should verify password");
        assert!(result, "Correct password should verify");
    }

    #[test]
    fn test_verify_password_incorrect() {
        let password = "SecurePass123!";
        let hash = hash_password(password).expect("Should hash password");

        let result = verify_password("WrongPassword123!", &hash).expect("Should verify password");
        assert!(!result, "Incorrect password should not verify");
    }

    #[test]
    fn test_verify_password_invalid_hash() {
        let result = verify_password("password", "invalid_hash_format");
        assert!(result.is_err(), "Invalid hash format should return error");
    }
}
