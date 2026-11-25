//! The device password rule: one rule for every node model, checked when a user sets it.
//!
//! The device password is a person's login password on whatever lab nodes they start, so it must
//! work on the strictest model they might use next. The rule is the intersection of the models'
//! own rules: 12 to 20 characters (cloudbase-init's limit is 20), only characters every node
//! configuration takes unquoted, every character class the strength checks of Firepower Threat
//! Defense, PAN-OS, ISE, Windows and NX-OS ask for, neither the username nor `cisco` inside it,
//! and no character three times in a row (Firepower Threat Defense). Each part has its own
//! message, and none repeats the password.

use std::fmt;

use anyhow::{Result, bail};

use crate::services::error::ServiceError;

/// The fewest characters a device password has.
const MIN_LENGTH: usize = 12;
/// The most characters a device password has.
const MAX_LENGTH: usize = 20;
/// The punctuation a device password may use, and must use one of.
const PUNCTUATION: &[char] = &['!', '@', '_', '-', '+', '.'];
/// A word no device password may contain, in any case: Cisco devices refuse it.
const FORBIDDEN_WORD: &str = "cisco";

/// The part of the device password rule a password breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePasswordRule {
    Length { length: usize },
    Characters,
    FirstCharacter,
    Lowercase,
    Uppercase,
    Digit,
    Punctuation,
    Username { username: String },
    ForbiddenWord,
    Repeat,
}

impl fmt::Display for DevicePasswordRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length { length } => write!(
                f,
                "A device password has {MIN_LENGTH} to {MAX_LENGTH} characters; this one has {length}"
            ),
            Self::Characters => write!(
                f,
                "A device password has only letters, digits and the punctuation {}: no spaces, \
                 quotes or other symbols",
                punctuation_list()
            ),
            Self::FirstCharacter => write!(f, "A device password starts with a letter or a digit"),
            Self::Lowercase => write!(f, "A device password has at least one lowercase letter"),
            Self::Uppercase => write!(f, "A device password has at least one uppercase letter"),
            Self::Digit => write!(f, "A device password has at least one digit"),
            Self::Punctuation => write!(
                f,
                "A device password has at least one of the punctuation characters {}",
                punctuation_list()
            ),
            Self::Username { username } => write!(
                f,
                "A device password does not contain your username '{username}'"
            ),
            Self::ForbiddenWord => write!(
                f,
                "A device password does not contain the word '{FORBIDDEN_WORD}', in any case"
            ),
            Self::Repeat => write!(f, "A device password has no character three times in a row"),
        }
    }
}

/// Check `password` against the device password rule, for the user `username`.
///
/// # Errors
///
/// `ServiceError::DevicePasswordRefused`, carrying the first part of the rule the password
/// breaks.
pub fn validate_device_password(password: &str, username: &str) -> Result<()> {
    let length = password.chars().count();
    if !(MIN_LENGTH..=MAX_LENGTH).contains(&length) {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::Length { length }
        });
    }
    if !password
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || PUNCTUATION.contains(&c))
    {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::Characters
        });
    }
    if !password
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::FirstCharacter
        });
    }
    if !password.chars().any(|c| c.is_ascii_lowercase()) {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::Lowercase
        });
    }
    if !password.chars().any(|c| c.is_ascii_uppercase()) {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::Uppercase
        });
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::Digit
        });
    }
    if !password.chars().any(|c| PUNCTUATION.contains(&c)) {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::Punctuation
        });
    }
    let lowered = password.to_ascii_lowercase();
    if lowered.contains(&username.to_ascii_lowercase()) {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::Username {
                username: username.to_owned(),
            }
        });
    }
    if lowered.contains(FORBIDDEN_WORD) {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::ForbiddenWord
        });
    }
    if password
        .as_bytes()
        .windows(3)
        .any(|run| run[0] == run[1] && run[1] == run[2])
    {
        bail!(ServiceError::DevicePasswordRefused {
            rule: DevicePasswordRule::Repeat
        });
    }
    Ok(())
}

/// The allowed punctuation, for messages: `! @ _ - + .`.
fn punctuation_list() -> String {
    PUNCTUATION
        .iter()
        .map(char::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const USERNAME: &str = "alice";

    /// A password that meets every part of the rule.
    const GOOD: &str = "Granite-Owl42";

    /// `password` is refused for breaking `expected`.
    fn refused(password: &str, expected: &DevicePasswordRule) {
        let error = validate_device_password(password, USERNAME).expect_err(password);
        assert_eq!(
            error.downcast_ref::<ServiceError>(),
            Some(&ServiceError::DevicePasswordRefused {
                rule: expected.clone()
            }),
            "{password}"
        );
    }

    fn accepted(password: &str) {
        validate_device_password(password, USERNAME).expect(password);
    }

    #[test]
    fn device_password_006_only_letters_digits_and_the_listed_punctuation_starting_alphanumeric() {
        accepted(GOOD);
        // Every allowed punctuation character, at either length limit.
        accepted("Ab1!@_-+.xyz");
        accepted("Zz9.Qq8+Ww7-Ee6_Rr5@");
        accepted("Granite.Owl42");
        for bad in [
            "Granite Owl42",
            "Granite'Owl42",
            r#"Granite"Owl42"#,
            "Granite\\Owl42",
            "Granite$Owl42",
            "Granite#Owl42",
            "Granite?Owl42",
            "Granite;Owl42",
            "Granite%Owl42",
            "Granite&Owl42",
            "Granité-Owl42",
        ] {
            refused(bad, &DevicePasswordRule::Characters);
        }
        accepted("9Granite-Owl4");
        refused("-Granite-Owl4", &DevicePasswordRule::FirstCharacter);
    }

    #[test]
    fn device_password_007_it_has_a_lowercase_an_uppercase_a_digit_and_a_punctuation_character() {
        accepted(GOOD);
        refused("GRANITE-OWL42", &DevicePasswordRule::Lowercase);
        refused("granite-owl42", &DevicePasswordRule::Uppercase);
        refused("Granite-Owlxy", &DevicePasswordRule::Digit);
        refused("GraniteOwl42x", &DevicePasswordRule::Punctuation);
    }

    #[test]
    fn device_password_008_no_username_no_cisco_and_no_character_three_times_in_a_row() {
        accepted(GOOD);
        refused(
            "My-ALICE-pw42",
            &DevicePasswordRule::Username {
                username: USERNAME.to_owned(),
            },
        );
        refused("My-CiScO-pw42", &DevicePasswordRule::ForbiddenWord);
        accepted("Graanite-Owl42");
        refused("Graaanite-Owl4", &DevicePasswordRule::Repeat);
        refused("Granite-Owl444", &DevicePasswordRule::Repeat);
    }
}
