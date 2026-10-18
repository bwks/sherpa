use anyhow::{Context, Result, anyhow};
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
    let special_chars =
        Regex::new(r"[!@#$%^&*_+\-=]").context("Failed to compile password regex")?;
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
    fn test_validate_password_strength_error_lists_all_failures() {
        // "a" fails: too short, no uppercase, no special
        let err = validate_password_strength("a").unwrap_err().to_string();
        assert!(err.contains("Minimum 8 characters"));
        assert!(err.contains("uppercase"));
        assert!(err.contains("special character"));
        // Should NOT complain about lowercase since "a" has lowercase
        assert!(!err.contains("lowercase"));
    }

    #[test]
    fn test_validate_password_strength_boundary_length() {
        // Exactly 8 characters with all requirements met
        assert!(validate_password_strength("Abcdef1!").is_ok());
        // 7 characters — one too short
        assert!(validate_password_strength("Abcde1!").is_err());
    }
}
