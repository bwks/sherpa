//! What the record reads of a message's kind (its name in the `kind` columns, and which side
//! of a task sends it) and of a reference's kind.

use anyhow::{Result, bail};

use crate::message::{Kind, RefKind};

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

    /// The kind the record stores as `text`.
    ///
    /// # Errors
    ///
    /// Fails when `text` names no kind.
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "task" => Self::Task,
            "answer" => Self::Answer,
            "progress" => Self::Progress,
            "question" => Self::Question,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancel" => Self::Cancel,
            "canceled" => Self::Canceled,
            other => bail!("'{other}' is not a kind of message"),
        })
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

impl RefKind {
    /// The kind as the record stores it, such as `pull_request`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::PullRequest => "pull_request",
            Self::Branch => "branch",
        }
    }

    /// The kind the record stores as `text`.
    ///
    /// # Errors
    ///
    /// Fails when `text` names no kind.
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "issue" => Self::Issue,
            "pull_request" => Self::PullRequest,
            "branch" => Self::Branch,
            other => bail!("'{other}' is not a kind of reference"),
        })
    }
}
