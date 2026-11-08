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
    fn a_missing_or_multiline_unicode_description_is_accepted() {
        check_lab_description(None).unwrap();
        check_lab_description(Some("")).unwrap();
        check_lab_description(Some("Practice BGP.\r\n\tTwo routers, café ☕")).unwrap();
        check_lab_description(Some(&"é".repeat(MAX_DESCRIPTION_CHARS))).unwrap();
    }

    #[test]
    fn a_description_over_the_limit_is_rejected() {
        let error = check_lab_description(Some(&"a".repeat(MAX_DESCRIPTION_CHARS + 1)))
            .unwrap_err()
            .to_string();
        assert!(error.contains("1001 characters"), "{error}");
    }

    #[test]
    fn control_characters_other_than_line_breaks_and_tabs_are_rejected() {
        for description in ["bell\u{7}", "escape\u{1b}[31m", "nul\u{0}"] {
            let error = check_lab_description(Some(description))
                .unwrap_err()
                .to_string();
            assert!(error.contains("control character"), "{error}");
        }
    }
}
