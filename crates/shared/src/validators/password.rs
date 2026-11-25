use std::fmt;

use anyhow::Result;

use crate::SharedError;

/// The special characters a password can use to meet the special character rule.
const PASSWORD_SPECIAL_CHARACTERS: &str = "!@#$%^&*_+-=";

/// A part of a strong password.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordRequirement {
    MinimumLength,
    Uppercase,
    Lowercase,
    SpecialCharacter,
}

impl fmt::Display for PasswordRequirement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MinimumLength => write!(f, "Minimum 8 characters"),
            Self::Uppercase => write!(f, "At least one uppercase letter (A-Z)"),
            Self::Lowercase => write!(f, "At least one lowercase letter (a-z)"),
            Self::SpecialCharacter => write!(f, "At least one special character (!@#$%^&*_+-=)"),
        }
    }
}

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
/// `SharedError::WeakPassword`, carrying every requirement the password lacks.
pub fn validate_password_strength(password: &str) -> Result<()> {
    let mut lacking = Vec::new();
    if password.len() < 8 {
        lacking.push(PasswordRequirement::MinimumLength);
    }
    if !password.chars().any(|c| c.is_ascii_uppercase()) {
        lacking.push(PasswordRequirement::Uppercase);
    }
    if !password.chars().any(|c| c.is_ascii_lowercase()) {
        lacking.push(PasswordRequirement::Lowercase);
    }
    if !password
        .chars()
        .any(|c| PASSWORD_SPECIAL_CHARACTERS.contains(c))
    {
        lacking.push(PasswordRequirement::SpecialCharacter);
    }
    if !lacking.is_empty() {
        return Err(SharedError::WeakPassword { lacking }.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::auth::password::hash_password;

    #[test]
    fn user_passwords_008_a_password_needs_the_four_parts_and_a_refusal_carries_what_is_lacking() {
        let valid_passwords = vec![
            "Password123!",
            "SecureP@ss1",
            "MyP@ssw0rd",
            "Complex#Pass123",
        ];

        for password in valid_passwords {
            assert!(
                validate_password_strength(password).is_ok(),
                "Password '{password}' should be valid"
            );
        }

        assert!(validate_password_strength("password123!").is_err());
        assert!(validate_password_strength("PASSWORD123!").is_err());
        assert!(validate_password_strength("Password123").is_err());

        let error = validate_password_strength("a").unwrap_err();
        assert_eq!(
            error.downcast_ref::<SharedError>(),
            Some(&SharedError::WeakPassword {
                lacking: vec![
                    PasswordRequirement::MinimumLength,
                    PasswordRequirement::Uppercase,
                    PasswordRequirement::SpecialCharacter,
                ],
            })
        );

        // Exactly 8 characters with all requirements met
        assert!(validate_password_strength("Abcdef1!").is_ok());
        // 7 characters — one too short
        assert!(validate_password_strength("Abcde1!").is_err());

        assert!(hash_password("weak").is_err());
    }
}
