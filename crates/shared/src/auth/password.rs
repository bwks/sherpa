use anyhow::{Result, anyhow};
use argon2::password_hash::Error as HashError;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};

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

    // Hash the password with Argon2id (default parameters) and a random 16-byte salt from the
    // operating system
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes())
        .map_err(|e| anyhow!("Failed to hash password: {e}"))?
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

    // Only a mismatch is a wrong password; a hash argon2 cannot check is an error to report.
    match argon2.verify_password(password.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(HashError::PasswordInvalid) => Ok(false),
        Err(error) => Err(anyhow!("Cannot check the password hash: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_passwords_010_a_hash_is_argon2id_salted_and_verifies_only_the_right_password() {
        let password = "SecurePass123!";
        let hash1 = hash_password(password).expect("Should hash password");
        let hash2 = hash_password(password).expect("Should hash password");

        // Same password should produce different hashes due to random salt
        assert_ne!(hash1, hash2);
        assert!(
            hash1.starts_with("$argon2id$v=19$"),
            "unexpected parameters in {hash1}"
        );

        let result = verify_password(password, &hash1).expect("Should verify password");
        assert!(result, "Correct password should verify");
    }

    #[test]
    fn user_passwords_011_a_hash_stored_by_argon2_0_5_still_verifies() {
        let stored = "$argon2id$v=19$m=19456,t=2,p=1$2FJMSCOFArFldYJuaUcZXg$FsJQ0y+UxlbmOc7owiv4HqEGA6KuBQC2uIHm3NA2UQ8";
        assert!(verify_password("StoredPass123!", stored).expect("Should verify password"));
        assert!(!verify_password("WrongPass123!", stored).expect("Should verify password"));
    }

    #[test]
    fn user_passwords_012_a_hash_that_cannot_be_checked_is_an_error_not_a_wrong_password() {
        let scrypt = "$scrypt$ln=4,r=8,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYXNoaGFzaA";
        assert!(verify_password("TestPass123!", scrypt).is_err());

        let result = verify_password("password", "invalid_hash_format");
        assert!(result.is_err(), "Invalid hash format should return error");
    }
}
