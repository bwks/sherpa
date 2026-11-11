//! A task's row, as its messages update it. This is the logic of the `task` table, with no I/O:
//! `write::record` reads the row, applies the message and writes it back.

use anyhow::{Result, bail};
use serde::Serialize;

use crate::message::{Kind, Message, TaskUsage};

/// Where a task is, from the last of its messages that says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// The task was sent; nothing came back yet.
    Sent,
    /// Its agent reported progress, or an answer was sent.
    Running,
    /// Its agent asked a question and waits for the answer.
    Question,
    Completed,
    Failed,
    Canceled,
}

/// One message as read from the `_record` consumer: where the stream keeps it and when it stored it.
#[derive(Debug, Clone, Copy)]
pub struct Recorded<'a> {
    /// The stream sequence.
    pub seq: u64,
    /// When the stream stored it, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    pub message: &'a Message,
}

/// A row of the `task` table.
#[derive(Debug, Clone, PartialEq)]
pub struct Task {
    pub id: String,
    /// The agent working on the task.
    pub agent: String,
    /// The agent that sent it.
    pub requester: String,
    pub first_seq: u64,
    pub last_seq: u64,
    pub last_at_ms: u64,
    /// The model the task message asked for.
    pub requested_model: Option<String>,
    pub github: bool,
    /// The task message's text; `None` for a task first seen through a later message.
    pub prompt: Option<String>,
    pub sent_at_ms: Option<u64>,
    /// The first progress.
    pub started_at_ms: Option<u64>,
    /// The last completed, failed or canceled.
    pub finished_at_ms: Option<u64>,
    pub state: TaskState,
    /// The model the agent says it ran on, from its last outcome that named one.
    pub model: Option<String>,
    /// The text of the last question or outcome.
    pub result: Option<String>,
    pub questions: u64,
    pub answers: u64,
    /// The last `task_usage` an outcome carried.
    pub usage: Option<TaskUsage>,
}

impl TaskState {
    /// The state as the database stores it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sent => "sent",
            Self::Running => "running",
            Self::Question => "question",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
        }
    }

    /// The state the database stores as `text`.
    ///
    /// # Errors
    ///
    /// Fails when `text` names no state.
    pub fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "sent" => Self::Sent,
            "running" => Self::Running,
            "question" => Self::Question,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "canceled" => Self::Canceled,
            other => bail!("'{other}' is not a task state"),
        })
    }
}

impl Task {
    /// The row of a task seen first in `recorded`, before it is applied.
    fn first_seen(recorded: &Recorded<'_>) -> Self {
        let message = recorded.message;
        let (agent, requester) = if message.kind.sent_by_worker() {
            (message.from.clone(), message.to.clone())
        } else {
            (message.to.clone(), message.from.clone())
        };
        Self {
            id: message.task.clone(),
            agent,
            requester,
            first_seq: recorded.seq,
            last_seq: recorded.seq,
            last_at_ms: recorded.at_ms,
            requested_model: None,
            github: false,
            prompt: None,
            sent_at_ms: None,
            started_at_ms: None,
            finished_at_ms: None,
            state: TaskState::Sent,
            model: None,
            result: None,
            questions: 0,
            answers: 0,
            usage: None,
        }
    }
}

/// `task` (`None` when the task has no row yet) with the message in `recorded` applied. Each
/// message is applied once: recording skips a sequence it already holds.
///
/// What a message says about where the task is (its state, result, model and usage) counts only
/// when it is newer than every message applied before, so a message delivered out of order
/// cannot take the task back. What it adds (its prompt, a question or answer counted, the
/// first progress) counts whatever its order.
pub fn apply(task: Option<Task>, recorded: &Recorded<'_>) -> Task {
    let message = recorded.message;
    let newer = task
        .as_ref()
        .is_none_or(|task| recorded.seq > task.last_seq);
    let mut task = task.unwrap_or_else(|| Task::first_seen(recorded));
    task.first_seq = task.first_seq.min(recorded.seq);
    task.last_seq = task.last_seq.max(recorded.seq);
    task.last_at_ms = task.last_at_ms.max(recorded.at_ms);
    match message.kind {
        Kind::Task => {
            task.prompt = Some(message.text.clone());
            task.requested_model.clone_from(&message.model);
            task.github = message.github;
            task.sent_at_ms = Some(recorded.at_ms);
        }
        Kind::Answer => {
            task.answers = task.answers.saturating_add(1);
            if newer {
                task.state = TaskState::Running;
            }
        }
        Kind::Progress => {
            task.started_at_ms = Some(
                task.started_at_ms
                    .map_or(recorded.at_ms, |at| at.min(recorded.at_ms)),
            );
            if newer && task.state == TaskState::Sent {
                task.state = TaskState::Running;
            }
        }
        Kind::Question => {
            task.questions = task.questions.saturating_add(1);
        }
        Kind::Cancel | Kind::Completed | Kind::Failed | Kind::Canceled => {}
    }
    if newer {
        apply_outcome(&mut task, recorded);
    }
    task
}

/// What an outcome or a question says about where `task` is.
fn apply_outcome(task: &mut Task, recorded: &Recorded<'_>) {
    let message = recorded.message;
    let state = match message.kind {
        Kind::Question => TaskState::Question,
        Kind::Completed => TaskState::Completed,
        Kind::Failed => TaskState::Failed,
        Kind::Canceled => TaskState::Canceled,
        Kind::Task | Kind::Answer | Kind::Progress | Kind::Cancel => return,
    };
    task.state = state;
    task.result = Some(message.text.clone());
    if state != TaskState::Question {
        task.finished_at_ms = Some(recorded.at_ms);
    }
    if message.model.is_some() {
        task.model.clone_from(&message.model);
    }
    if message.task_usage.is_some() {
        task.usage.clone_from(&message.task_usage);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(kind: Kind, text: &str) -> Message {
        let (from, to) = if kind.sent_by_worker() {
            ("dev", "host")
        } else {
            ("host", "dev")
        };
        Message {
            task: "t1".to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            kind,
            text: text.to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
        }
    }

    fn used(runs: u64, cost_usd: f64) -> TaskUsage {
        TaskUsage {
            runs,
            cost_usd,
            ..TaskUsage::default()
        }
    }

    /// Apply `messages` in order, the first at sequence 10 and one second apart.
    fn replay(messages: &[Message]) -> Task {
        let mut task = None;
        for (index, message) in (10_u64..).zip(messages) {
            task = Some(apply(
                task,
                &Recorded {
                    seq: index,
                    at_ms: index * 1000,
                    message,
                },
            ));
        }
        task.unwrap()
    }

    #[test]
    fn a_task_goes_from_sent_through_a_question_to_completed() {
        let mut task_message = message(Kind::Task, "do it");
        task_message.model = Some("claude-sonnet-5-5".to_owned());
        task_message.github = true;
        let mut question = message(Kind::Question, "which?");
        question.task_usage = Some(used(1, 0.25));
        let mut done = message(Kind::Completed, "done");
        done.model = Some("claude-sonnet-5-5".to_owned());
        done.task_usage = Some(used(2, 0.5));

        let task = replay(&[task_message.clone()]);
        assert_eq!(task.state, TaskState::Sent);
        assert_eq!(task.agent, "dev");
        assert_eq!(task.requester, "host");
        assert_eq!(task.prompt.as_deref(), Some("do it"));
        assert_eq!(task.requested_model.as_deref(), Some("claude-sonnet-5-5"));
        assert!(task.github);
        assert_eq!(task.sent_at_ms, Some(10_000));

        let task = replay(&[
            task_message.clone(),
            message(Kind::Progress, "reading"),
            question.clone(),
        ]);
        assert_eq!(task.state, TaskState::Question);
        assert_eq!(task.started_at_ms, Some(11_000));
        assert_eq!(task.questions, 1);
        assert_eq!(task.usage, Some(used(1, 0.25)));
        assert_eq!(task.finished_at_ms, None);

        let task = replay(&[
            task_message,
            message(Kind::Progress, "reading"),
            question,
            message(Kind::Answer, "this"),
            message(Kind::Progress, "writing"),
            done,
        ]);
        assert_eq!(task.state, TaskState::Completed);
        assert_eq!(task.answers, 1);
        assert_eq!(task.started_at_ms, Some(11_000));
        assert_eq!(task.finished_at_ms, Some(15_000));
        assert_eq!(task.result.as_deref(), Some("done"));
        assert_eq!(task.model.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(task.usage, Some(used(2, 0.5)));
        assert_eq!((task.first_seq, task.last_seq), (10, 15));
        assert_eq!(task.last_at_ms, 15_000);
    }

    #[test]
    fn an_answer_sets_the_task_running_again() {
        let task = replay(&[
            message(Kind::Task, "do it"),
            message(Kind::Question, "which?"),
            message(Kind::Answer, "this"),
        ]);
        assert_eq!(task.state, TaskState::Running);
    }

    #[test]
    fn progress_after_the_outcome_does_not_reopen_the_task() {
        let task = replay(&[
            message(Kind::Task, "do it"),
            message(Kind::Completed, "done"),
            message(Kind::Progress, "late"),
        ]);
        assert_eq!(task.state, TaskState::Completed);
    }

    #[test]
    fn a_task_first_seen_through_its_outcome_has_no_prompt() {
        let mut done = message(Kind::Failed, "broke");
        done.task_usage = Some(used(1, 0.1));
        let task = replay(&[done]);
        assert_eq!(task.state, TaskState::Failed);
        assert_eq!(task.agent, "dev");
        assert_eq!(task.requester, "host");
        assert_eq!(task.prompt, None);
        assert_eq!(task.sent_at_ms, None);
        assert_eq!(task.usage, Some(used(1, 0.1)));
    }

    #[test]
    fn an_outcome_without_a_model_or_usage_keeps_the_ones_known() {
        let mut first = message(Kind::Question, "which?");
        first.model = Some("claude-haiku-5-5".to_owned());
        first.task_usage = Some(used(1, 0.1));
        let task = replay(&[first, message(Kind::Canceled, "stopped")]);
        assert_eq!(task.state, TaskState::Canceled);
        assert_eq!(task.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(task.usage, Some(used(1, 0.1)));
    }

    #[test]
    fn an_older_message_delivered_late_does_not_take_the_task_back() {
        let mut done = message(Kind::Completed, "done");
        done.task_usage = Some(used(2, 0.5));
        let task = replay(&[message(Kind::Task, "do it"), done]);
        let mut question = message(Kind::Question, "which?");
        question.task_usage = Some(used(1, 0.25));
        let task = apply(
            Some(task),
            &Recorded {
                seq: 5,
                at_ms: 5_000,
                message: &question,
            },
        );
        assert_eq!(task.state, TaskState::Completed);
        assert_eq!(task.usage, Some(used(2, 0.5)));
        assert_eq!(task.result.as_deref(), Some("done"));
        // It still counts, and moves the first sequence back.
        assert_eq!(task.questions, 1);
        assert_eq!((task.first_seq, task.last_seq), (5, 11));
    }

    #[test]
    fn a_cancel_request_changes_only_the_last_sequence() {
        let before = replay(&[message(Kind::Task, "do it")]);
        let after = replay(&[message(Kind::Task, "do it"), message(Kind::Cancel, "")]);
        assert_eq!(after.state, before.state);
        assert_eq!(after.last_seq, 11);
    }

    #[test]
    fn every_state_reads_back_from_its_text() {
        for state in [
            TaskState::Sent,
            TaskState::Running,
            TaskState::Question,
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Canceled,
        ] {
            assert_eq!(TaskState::parse(state.as_str()).unwrap(), state);
            assert_eq!(serde_json::to_value(state).unwrap(), state.as_str());
        }
        assert!(TaskState::parse("done").is_err());
    }
}
