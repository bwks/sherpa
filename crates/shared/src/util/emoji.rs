use crate::konst::{EMOJI_BAD, EMOJI_GOOD, EMOJI_WARN};

// ============================================================================
// Emoji Helper Functions
// ============================================================================

/// Formats a message with a success emoji (✅) prefix.
///
/// # Example
/// ```
/// use shared::konst::emoji_success;
/// let msg = emoji_success("Operation completed");
/// assert_eq!(msg, "✅ Operation completed");
/// ```
pub fn emoji_success(msg: &str) -> String {
    format!("{} {}", EMOJI_GOOD, msg)
}

/// Formats a message with an error emoji (❌) prefix.
///
/// # Example
/// ```
/// use shared::konst::emoji_error;
/// let msg = emoji_error("Operation failed");
/// assert_eq!(msg, "❌ Operation failed");
/// ```
pub fn emoji_error(msg: &str) -> String {
    format!("{} {}", EMOJI_BAD, msg)
}

/// Formats a message with a warning emoji (⚠️) prefix.
///
/// # Example
/// ```
/// use shared::konst::emoji_warning;
/// let msg = emoji_warning("Caution advised");
/// assert_eq!(msg, "⚠️ Caution advised");
/// ```
pub fn emoji_warning(msg: &str) -> String {
    format!("{} {}", EMOJI_WARN, msg)
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emoji_success() {
        let result = emoji_success("Test passed");
        assert_eq!(result, "✅ Test passed");
        assert!(result.starts_with(EMOJI_GOOD));
    }

    #[test]
    fn test_emoji_error() {
        let result = emoji_error("Test failed");
        assert_eq!(result, "❌ Test failed");
        assert!(result.starts_with(EMOJI_BAD));
    }

    #[test]
    fn test_emoji_warning() {
        let result = emoji_warning("Test warning");
        assert_eq!(result, "⚠️ Test warning");
        assert!(result.starts_with(EMOJI_WARN));
    }

    #[test]
    fn test_emoji_empty_string() {
        assert_eq!(emoji_success(""), "✅ ");
        assert_eq!(emoji_error(""), "❌ ");
        assert_eq!(emoji_warning(""), "⚠️ ");
    }

    #[test]
    fn test_emoji_with_special_chars() {
        let msg = "Test: 100% complete!";
        assert_eq!(emoji_success(msg), "✅ Test: 100% complete!");
        assert_eq!(emoji_error(msg), "❌ Test: 100% complete!");
        assert_eq!(emoji_warning(msg), "⚠️ Test: 100% complete!");
    }
}
