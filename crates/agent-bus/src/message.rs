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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// The task this message belongs to.
    pub task: String,
    pub from: String,
    pub to: String,
    pub kind: Kind,
    pub text: String,
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
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["kind"], "question");
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);
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
