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
    Success,

    Error,

    Warning,

    Info,

    Progress,

    Rocket,

    Stop,

    Hourglass,

    Question,

    Lock,

    Unlock,

    Fire,

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
#[must_use]
pub fn emoji_success(msg: &str) -> String {
    format!("{} {}", Emoji::Success, msg)
}

/// Formats a message with a warning emoji (⚠️) prefix.
///
#[must_use]
pub fn emoji_warning(msg: &str) -> String {
    format!("{} {}", Emoji::Warning, msg)
}
