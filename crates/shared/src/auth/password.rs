use anyhow::{Context, Result, anyhow};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rand::rngs::OsRng;
use regex::Regex;

/// Validates password strength requirements.
///
/// Requirements:
/// - Minimum 8 characters
/// - At least one uppercase letter (A-Z)
/// - At least one lowercase letter (a-z)
/// - At least one special character (!@#$%^&*_+-=)
///
/// # Errors
///
/// Returns an error with a descriptive message if the password does not meet requirements.
pub fn validate_password_strength(password: &str) -> Result<()> {
    let mut errors = Vec::new();

    // Check minimum length
    if password.len() < 8 {
        errors.push("• Minimum 8 characters");
    }

    // Check for uppercase letter
    if !password.chars().any(|c| c.is_ascii_uppercase()) {
        errors.push("• At least one uppercase letter (A-Z)");
    }

    // Check for lowercase letter
    if !password.chars().any(|c| c.is_ascii_lowercase()) {
        errors.push("• At least one lowercase letter (a-z)");
    }

    // Check for special character (restricted safe set)
    let special_chars = Regex::new(r"[!@#$%^&*_+\-=]").unwrap();
    if !special_chars.is_match(password) {
        errors.push("• At least one special character (!@#$%^&*_+-=)");
    }

    if !errors.is_empty() {
        return Err(anyhow!(
            "Password does not meet security requirements:\n{}",
            errors.join("\n")
        ));
    }

    Ok(())
}

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
    let parsed_hash = PasswordHash::new(hash).context("Invalid password hash format")?;

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
    fn test_validate_password_strength_valid() {
        let valid_passwords = vec![
            "Password123!",
            "SecureP@ss1",
            "MyP@ssw0rd",
            "Complex#Pass123",
        ];

        for password in valid_passwords {
            assert!(
                validate_password_strength(password).is_ok(),
                "Password '{}' should be valid",
                password
            );
        }
    }

    #[test]
    fn test_validate_password_strength_too_short() {
        assert!(validate_password_strength("Pass1!").is_err());
    }

    #[test]
    fn test_validate_password_strength_no_uppercase() {
        assert!(validate_password_strength("password123!").is_err());
    }

    #[test]
    fn test_validate_password_strength_no_lowercase() {
        assert!(validate_password_strength("PASSWORD123!").is_err());
    }

    #[test]
    fn test_validate_password_strength_no_special() {
        assert!(validate_password_strength("Password123").is_err());
    }

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
