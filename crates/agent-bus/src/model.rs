//! The models Claude Code runs on: the aliases a task may name instead of a full ID, mapped to
//! the current IDs here and nowhere else.

use anyhow::{Result, ensure};

/// The current ID of each model family.
const OPUS: &str = "claude-opus-5-5";
const SONNET: &str = "claude-sonnet-5-5";
const HAIKU: &str = "claude-haiku-5-5";
/// The model guests run Claude Code on unless a task or `--model` says otherwise.
pub const DEFAULT_MODEL: &str = OPUS;
/// What a full model ID starts with.
const ID_PREFIX: &str = "claude-";
/// The longest model ID, in characters.
const MAX_ID: usize = 64;

/// The full model ID for `name`: an alias (`opus`, `sonnet`, `haiku`) becomes its current ID, and
/// a full ID such as `claude-sonnet-5-5` is kept as it is.
///
/// # Errors
///
/// Fails when `name` is neither an alias nor a full ID: `claude-` followed by lowercase letters,
/// digits, hyphens and dots, 64 characters at most.
pub fn model_id(name: &str) -> Result<String> {
    let id = match name {
        "opus" => OPUS,
        "sonnet" => SONNET,
        "haiku" => HAIKU,
        _ => name,
    };
    ensure!(
        id.len() > ID_PREFIX.len()
            && id.len() <= MAX_ID
            && id.starts_with(ID_PREFIX)
            && id.chars().all(|character| character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
                || character == '.'),
        "Model '{name}' must be opus, sonnet, haiku or a full ID such as {SONNET}"
    );
    Ok(id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_map_to_the_current_ids() {
        assert_eq!(model_id("opus").unwrap(), "claude-opus-5-5");
        assert_eq!(model_id("sonnet").unwrap(), "claude-sonnet-5-5");
        assert_eq!(model_id("haiku").unwrap(), "claude-haiku-5-5");
        assert_eq!(DEFAULT_MODEL, "claude-opus-5-5");
    }

    #[test]
    fn full_ids_are_kept() {
        for id in ["claude-sonnet-5-5", "claude-haiku-5-5", "claude-opus-5-5"] {
            assert_eq!(model_id(id).unwrap(), id);
        }
    }

    #[test]
    fn other_names_are_refused() {
        for name in [
            "",
            "Opus",
            "gpt-5",
            "claude-",
            "claude-opus 5",
            "--resume",
            "claude-opus-5-5;rm",
            &format!("claude-{}", "a".repeat(64)),
        ] {
            assert!(model_id(name).is_err(), "{name}");
        }
    }
}
