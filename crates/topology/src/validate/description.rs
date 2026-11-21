use anyhow::{Result, bail};

/// The longest lab description accepted, in characters.
const MAX_DESCRIPTION_CHARS: usize = 1000;

/// Check a lab's `description`: at most 1000 characters, and no control characters other than
/// line breaks and tabs, since every interface that shows a lab shows it.
///
/// # Errors
///
/// Fails when the description is too long or holds another control character.
pub fn check_lab_description(description: Option<&str>) -> Result<()> {
    let Some(description) = description else {
        return Ok(());
    };
    let length = description.chars().count();
    if length > MAX_DESCRIPTION_CHARS {
        bail!(
            "Manifest - `description` is {length} characters long; the limit is {MAX_DESCRIPTION_CHARS}"
        );
    }
    if let Some(character) = description
        .chars()
        .find(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        bail!(
            "Manifest - `description` holds the control character {character:?}; only line breaks and tabs are allowed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lab_settings_001_a_missing_empty_or_long_multiline_unicode_description_is_taken() {
        check_lab_description(None).unwrap();
        check_lab_description(Some("")).unwrap();
        check_lab_description(Some("Practice BGP.\r\n\tTwo routers, café ☕")).unwrap();
        check_lab_description(Some(&"é".repeat(MAX_DESCRIPTION_CHARS))).unwrap();
    }

    #[test]
    fn lab_settings_002_a_description_of_1001_characters_is_refused_giving_its_length() {
        let error = check_lab_description(Some(&"a".repeat(MAX_DESCRIPTION_CHARS + 1)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("1001 characters"), "{error}");
    }

    #[test]
    fn lab_settings_003_a_control_character_other_than_a_line_break_or_tab_is_refused() {
        for description in ["bell\u{7}", "escape\u{1b}[31m", "nul\u{0}"] {
            let error = check_lab_description(Some(description))
                .unwrap_err()
                .to_string();
            assert!(error.contains("control character"), "{error}");
        }
    }
}
