use std::fmt;

// ============================================================================
// Emoji Enum
// ============================================================================

/// Visual status indicators using emoji characters
///
/// Provides a type-safe way to use emoji in CLI output for status
/// indication, progress updates, and user feedback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Emoji {
    /// Success indicator (✅) - Operations completed successfully
    Success,

    /// Error indicator (❌) - Operations failed
    Error,

    /// Warning indicator (⚠️) - Caution, non-critical issues
    Warning,

    /// Information indicator (ℹ️) - Informational messages, tips
    Info,

    /// Progress indicator (🔄) - Operations in progress
    Progress,

    /// Rocket indicator (🚀) - Starting operations, deployment, launch
    Rocket,

    /// Stop indicator (🛑) - Stopped services, halted operations
    Stop,

    /// Hourglass indicator (⏳) - Waiting, pending, time-consuming operations
    Hourglass,

    /// Question indicator (❓) - Prompts, unclear status
    Question,

    /// Lock indicator (🔒) - Security, authentication, permissions
    Lock,

    /// Unlock indicator (🔓) - Unlocked, accessible
    Unlock,

    /// Fire indicator (🔥) - Critical issues, urgent attention needed
    Fire,

    /// Sparkles indicator (✨) - New features, highlights, special items
    Sparkles,
}

impl fmt::Display for Emoji {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Emoji::Success => write!(f, "✅"),
            Emoji::Error => write!(f, "❌"),
            Emoji::Warning => write!(f, "⚠️"),
            Emoji::Info => write!(f, "ℹ️"),
            Emoji::Progress => write!(f, "🔄"),
            Emoji::Rocket => write!(f, "🚀"),
            Emoji::Stop => write!(f, "🛑"),
            Emoji::Hourglass => write!(f, "⏳"),
            Emoji::Question => write!(f, "❓"),
            Emoji::Lock => write!(f, "🔒"),
            Emoji::Unlock => write!(f, "🔓"),
            Emoji::Fire => write!(f, "🔥"),
            Emoji::Sparkles => write!(f, "✨"),
        }
    }
}

// ============================================================================
// Emoji Helper Functions
// ============================================================================

/// Formats a message with a success emoji (✅) prefix.
///
pub fn emoji_success(msg: &str) -> String {
    format!("{} {}", Emoji::Success, msg)
}

/// Formats a message with an error emoji (❌) prefix.
///
pub fn emoji_error(msg: &str) -> String {
    format!("{} {}", Emoji::Error, msg)
}

/// Formats a message with a warning emoji (⚠️) prefix.
///
pub fn emoji_warning(msg: &str) -> String {
    format!("{} {}", Emoji::Warning, msg)
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
        assert!(result.starts_with(&Emoji::Success.to_string()));
    }

    #[test]
    fn test_emoji_error() {
        let result = emoji_error("Test failed");
        assert_eq!(result, "❌ Test failed");
        assert!(result.starts_with(&Emoji::Error.to_string()));
    }

    #[test]
    fn test_emoji_warning() {
        let result = emoji_warning("Test warning");
        assert_eq!(result, "⚠️ Test warning");
        assert!(result.starts_with(&Emoji::Warning.to_string()));
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
