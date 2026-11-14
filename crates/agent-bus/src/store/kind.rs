//! What the record reads of a message's kind: its name in the `kind` columns, and which side of
//! a task sends it.

use crate::message::Kind;

impl Kind {
    /// The kind as it is on the wire, such as `completed`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Answer => "answer",
            Self::Progress => "progress",
            Self::Question => "question",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancel => "cancel",
            Self::Canceled => "canceled",
        }
    }

    /// Whether the kind is one the agent working on a task sends: its progress, a question or
    /// an outcome. The others (a task, an answer, a cancel) go to that agent.
    pub fn sent_by_worker(self) -> bool {
        matches!(
            self,
            Self::Progress | Self::Question | Self::Completed | Self::Failed | Self::Canceled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_KINDS: [Kind; 8] = [
        Kind::Task,
        Kind::Answer,
        Kind::Progress,
        Kind::Question,
        Kind::Completed,
        Kind::Failed,
        Kind::Cancel,
        Kind::Canceled,
    ];

    #[test]
    fn a_kind_s_text_is_its_wire_name() {
        for kind in ALL_KINDS {
            assert_eq!(serde_json::to_value(kind).unwrap(), kind.as_str());
        }
    }

    #[test]
    fn only_the_working_agent_s_kinds_are_sent_by_the_worker() {
        let sent_by_worker: Vec<Kind> = ALL_KINDS
            .into_iter()
            .filter(|kind| kind.sent_by_worker())
            .collect();
        assert_eq!(
            sent_by_worker,
            [
                Kind::Progress,
                Kind::Question,
                Kind::Completed,
                Kind::Failed,
                Kind::Canceled
            ]
        );
    }
}
