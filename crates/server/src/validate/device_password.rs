//! The device password rule: one rule for every node model, checked when a user sets it.
//!
//! The device password is a person's login password on whatever lab nodes they start, so it must
//! work on the strictest model they might use next. The rule is the intersection of the models'
//! own rules: 12 to 20 characters (cloudbase-init's limit is 20), only characters every node
//! configuration takes unquoted, every character class the strength checks of Firepower Threat
//! Defense, PAN-OS, ISE, Windows and NX-OS ask for, neither the username nor `cisco` inside it,
//! and no character three times in a row (Firepower Threat Defense). Each part has its own
//! message, and none repeats the password.

use anyhow::{Result, bail};

/// The fewest characters a device password has.
const MIN_LENGTH: usize = 12;
/// The most characters a device password has.
const MAX_LENGTH: usize = 20;
/// The punctuation a device password may use, and must use one of.
const PUNCTUATION: &[char] = &['!', '@', '_', '-', '+', '.'];
/// A word no device password may contain, in any case: Cisco devices refuse it.
const FORBIDDEN_WORD: &str = "cisco";

/// Check `password` against the device password rule, for the user `username`.
///
/// # Errors
///
/// Fails with the first part of the rule the password breaks.
pub fn validate_device_password(password: &str, username: &str) -> Result<()> {
    let length = password.chars().count();
    if !(MIN_LENGTH..=MAX_LENGTH).contains(&length) {
        bail!(
            "A device password has {MIN_LENGTH} to {MAX_LENGTH} characters; this one has {length}"
        );
    }
    if !password
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || PUNCTUATION.contains(&c))
    {
        bail!(
            "A device password has only letters, digits and the punctuation {}: no spaces, \
             quotes or other symbols",
            punctuation_list()
        );
    }
    if !password
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        bail!("A device password starts with a letter or a digit");
    }
    if !password.chars().any(|c| c.is_ascii_lowercase()) {
        bail!("A device password has at least one lowercase letter");
    }
    if !password.chars().any(|c| c.is_ascii_uppercase()) {
        bail!("A device password has at least one uppercase letter");
    }
    if !password.chars().any(|c| c.is_ascii_digit()) {
        bail!("A device password has at least one digit");
    }
    if !password.chars().any(|c| PUNCTUATION.contains(&c)) {
        bail!(
            "A device password has at least one of the punctuation characters {}",
            punctuation_list()
        );
    }
    let lowered = password.to_ascii_lowercase();
    if lowered.contains(&username.to_ascii_lowercase()) {
        bail!("A device password does not contain your username '{username}'");
    }
    if lowered.contains(FORBIDDEN_WORD) {
        bail!("A device password does not contain the word '{FORBIDDEN_WORD}', in any case");
    }
    if password
        .as_bytes()
        .windows(3)
        .any(|run| run[0] == run[1] && run[1] == run[2])
    {
        bail!("A device password has no character three times in a row");
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

    /// `password` is refused with a message containing `expected`, and the message does not
    /// repeat the password.
    fn refused(password: &str, expected: &str) {
        let error = validate_device_password(password, USERNAME)
            .expect_err(password)
            .to_string();
        assert!(error.contains(expected), "{password}: {error}");
        assert!(!error.contains(password), "{password}: {error}");
    }

    fn accepted(password: &str) {
        validate_device_password(password, USERNAME).expect(password);
    }

    #[test]
    fn a_password_meeting_every_part_passes() {
        accepted(GOOD);
        // Every allowed punctuation character, at either length limit.
        accepted("Ab1!@_-+.xyz");
        accepted("Zz9.Qq8+Ww7-Ee6_Rr5@");
    }

    #[test]
    fn length_is_12_to_20_characters() {
        accepted("Granite-Ow1x");
        refused("Granite-Ow1", "12 to 20 characters; this one has 11");
        accepted("Granite-Owl42-Lynx9x");
        refused(
            "Granite-Owl42-Lynx9xy",
            "12 to 20 characters; this one has 21",
        );
    }

    #[test]
    fn only_letters_digits_and_the_listed_punctuation() {
        accepted("Granite.Owl42");
        for bad in [
            "Granite Owl42",
            "Granite'Owl42",
            "Granite\"Owl42",
            "Granite\\Owl42",
            "Granite$Owl42",
            "Granite#Owl42",
            "Granite?Owl42",
            "Granite;Owl42",
            "Granite%Owl42",
            "Granite&Owl42",
            "Granité-Owl42",
        ] {
            refused(bad, "only letters, digits and the punctuation ! @ _ - + .");
        }
    }

    #[test]
    fn the_first_character_is_a_letter_or_digit() {
        accepted("9Granite-Owl4");
        refused("-Granite-Owl4", "starts with a letter or a digit");
    }

    #[test]
    fn it_has_a_lowercase_letter() {
        accepted(GOOD);
        refused("GRANITE-OWL42", "at least one lowercase letter");
    }

    #[test]
    fn it_has_an_uppercase_letter() {
        accepted(GOOD);
        refused("granite-owl42", "at least one uppercase letter");
    }

    #[test]
    fn it_has_a_digit() {
        accepted(GOOD);
        refused("Granite-Owlxy", "at least one digit");
    }

    #[test]
    fn it_has_a_punctuation_character() {
        accepted(GOOD);
        refused(
            "GraniteOwl42x",
            "at least one of the punctuation characters",
        );
    }

    #[test]
    fn it_does_not_contain_the_username_in_any_case() {
        accepted(GOOD);
        let error = validate_device_password("My-ALICE-pw42", USERNAME)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("does not contain your username 'alice'"),
            "{error}"
        );
    }

    #[test]
    fn it_does_not_contain_cisco_in_any_case() {
        accepted(GOOD);
        refused("My-CiScO-pw42", "does not contain the word 'cisco'");
    }

    #[test]
    fn no_character_three_times_in_a_row() {
        accepted("Graanite-Owl42");
        refused("Graaanite-Owl4", "no character three times in a row");
        refused("Granite-Owl444", "no character three times in a row");
    }
}
