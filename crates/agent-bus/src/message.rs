//! The messages agents exchange: a task, its progress, questions and answers, and its outcome.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The longest agent name, in characters.
const MAX_AGENT_NAME: usize = 32;

/// What a message is for. The outcomes and `Question` follow A2A's task states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Work for the receiving agent.
    Task,
    /// The answer to a question the receiving agent asked.
    Answer,
    /// What the agent working on a task is doing.
    Progress,
    /// The task needs a decision before it can go on.
    Question,
    /// The task is done.
    Completed,
    /// The task could not be done.
    Failed,
}

/// One message on the bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// The task this message belongs to.
    pub task: String,
    pub from: String,
    pub to: String,
    pub kind: Kind,
    pub text: String,
    /// What the run cost. Only the outcome messages (completed, question, failed) carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

/// What one Claude Code run used, from the `result` event of its `stream-json` output.
///
/// The token counts are the run's totals across all its turns. The cache hit ratio is
/// `cache_read / (input + cache_creation + cache_read)`: see [`Usage::cache_hit_permille`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens that were neither read from nor written to the prompt cache.
    pub input: u64,
    pub output: u64,
    /// Input tokens written to the prompt cache: a cache miss that pays to fill the cache.
    pub cache_creation: u64,
    /// Input tokens read from the prompt cache: a cache hit.
    pub cache_read: u64,
    pub num_turns: u64,
    /// Wall-clock time of the run, in milliseconds.
    pub duration_ms: u64,
    /// Time spent in API calls, in milliseconds.
    pub duration_api_ms: u64,
    /// The API-equivalent cost of the run, in US dollars. It is NOT billed: guest agents run on
    /// the Claude subscription. Read it as a measure of efficiency, not as money spent.
    pub total_cost_usd: f64,
}

impl Usage {
    /// The share of input tokens served from the prompt cache, in tenths of a percent, rounded down (0 to
    /// 1000), or `None` when the run had no input tokens or the arithmetic overflows.
    pub fn cache_hit_permille(&self) -> Option<u64> {
        let total = self
            .input
            .checked_add(self.cache_creation)?
            .checked_add(self.cache_read)?;
        self.cache_read.checked_mul(1000)?.checked_div(total)
    }
}

/// A new task ID: eight hexadecimal digits.
pub fn new_task_id() -> String {
    Uuid::new_v4().simple().to_string()[..8].to_owned()
}

/// Check an agent name: it names the agent's subject on the bus, so it is 1 to 32 lowercase
/// letters, digits and hyphens.
///
/// # Errors
///
/// Fails when the name is empty, too long or holds another character.
pub fn check_agent_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= MAX_AGENT_NAME
            && name.chars().all(|character| character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'),
        "Agent name '{name}' must be 1 to {MAX_AGENT_NAME} lowercase letters, digits and hyphens"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_use_snake_case_kinds_on_the_wire() {
        let message = Message {
            task: "1a2b3c4d".to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind: Kind::Question,
            text: "Add the dependency?".to_owned(),
            usage: None,
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["kind"], "question");
        assert!(json.get("usage").is_none());
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);
    }

    #[test]
    fn usage_round_trips_on_the_wire() {
        let message = Message {
            task: "1a2b3c4d".to_owned(),
            from: "dev".to_owned(),
            to: "host".to_owned(),
            kind: Kind::Completed,
            text: "Done".to_owned(),
            usage: Some(Usage {
                input: 10,
                output: 20,
                cache_creation: 30,
                cache_read: 60,
                num_turns: 2,
                duration_ms: 1500,
                duration_api_ms: 1200,
                total_cost_usd: 0.25,
            }),
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["usage"]["cache_read"], 60);
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);
    }

    #[test]
    fn cache_hit_permille_is_cache_read_over_all_input() {
        let usage = Usage {
            input: 10,
            cache_creation: 30,
            cache_read: 60,
            ..Usage::default()
        };
        assert_eq!(usage.cache_hit_permille(), Some(600));
        assert_eq!(Usage::default().cache_hit_permille(), None);
        let overflow = Usage {
            cache_read: u64::MAX,
            input: 1,
            ..Usage::default()
        };
        assert_eq!(overflow.cache_hit_permille(), None);
    }

    #[test]
    fn task_ids_are_eight_hex_digits() {
        let id = new_task_id();
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|character| character.is_ascii_hexdigit()));
        assert_ne!(new_task_id(), id);
    }

    #[test]
    fn agent_names_must_be_valid_subject_tokens() {
        for name in ["dev", "host", "guest-2"] {
            check_agent_name(name).unwrap();
        }
        for name in ["", "Dev", "a.b", "a*", "a b", &"a".repeat(33)] {
            assert!(check_agent_name(name).is_err(), "{name}");
        }
    }
}
