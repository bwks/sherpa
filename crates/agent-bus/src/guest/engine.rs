//! The guest agent's decisions: what a message means given what it is doing, and which Claude
//! session a task has. Pure logic.

use std::collections::VecDeque;

use crate::message::{Kind, Message};

/// A Claude session, and whether its task may use GitHub.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Session {
    pub(super) id: String,
    pub(super) github: bool,
}

/// What a message arriving at the guest agent means, given what it is doing.
#[derive(Debug, PartialEq)]
pub(super) enum Control {
    /// A task or an answer: run it after the ones before it.
    Queue,
    /// A cancel for the task that is running.
    CancelRunning,
    /// A cancel for a task waiting in the queue.
    CancelQueued,
    /// A cancel for a task that is neither running nor queued.
    CancelUnknown,
    /// A message guest agents do not take.
    Ignore,
}

/// What a message means: tasks and answers queue, a cancel finds its task running, queued or
/// unknown. `running` is the task Claude Code works on now.
pub(super) fn classify(
    message: &Message,
    running: Option<&str>,
    queue: &VecDeque<Message>,
) -> Control {
    match message.kind {
        Kind::Task | Kind::Answer => Control::Queue,
        Kind::Cancel => {
            if running == Some(message.task.as_str()) {
                Control::CancelRunning
            } else if queue.iter().any(|queued| queued.task == message.task) {
                Control::CancelQueued
            } else {
                Control::CancelUnknown
            }
        }
        Kind::Progress | Kind::Question | Kind::Completed | Kind::Failed | Kind::Canceled => {
            Control::Ignore
        }
    }
}

/// The text of a `canceled` reply: the progress so far, and whether the run's cost is known.
/// It is known only when Claude Code's `result` event arrived before the cancel.
pub(super) fn canceled_text(last: Option<String>, cost_known: bool) -> String {
    let progress = match last {
        Some(last) => format!("Canceled. Last progress: {last}"),
        None => "Canceled before Claude Code reported progress".to_owned(),
    };
    if cost_known {
        progress
    } else {
        format!("{progress}. The cost of this run is unknown: no result event arrived")
    }
}

/// Whether a task may use GitHub: a task says so itself, and an answer has the flag of the task
/// it answers, whatever the answer says.
pub(super) fn task_github(message: &Message, session: Option<&Session>) -> bool {
    match message.kind {
        Kind::Answer => session.is_some_and(|session| session.github),
        _ => message.github,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task_message(kind: Kind, github: bool) -> Message {
        Message {
            task: "t1".to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind,
            text: String::new(),
            usage: None,
            task_usage: None,
            github,
        }
    }

    fn queued(kind: Kind, task: &str) -> Message {
        Message {
            task: task.to_owned(),
            ..task_message(kind, false)
        }
    }

    #[test]
    fn an_answer_inherits_the_flag_of_its_task_and_cannot_set_it() {
        let with = Session {
            id: "s".to_owned(),
            github: true,
        };
        let without = Session {
            id: "s".to_owned(),
            github: false,
        };
        assert!(task_github(&task_message(Kind::Task, true), None));
        assert!(!task_github(&task_message(Kind::Task, false), None));
        assert!(task_github(&task_message(Kind::Answer, false), Some(&with)));
        assert!(!task_github(
            &task_message(Kind::Answer, true),
            Some(&without)
        ));
        assert!(!task_github(&task_message(Kind::Answer, true), None));
    }

    #[test]
    fn tasks_and_answers_queue_whatever_runs() {
        let queue = VecDeque::new();
        for kind in [Kind::Task, Kind::Answer] {
            let message = queued(kind, "t1");
            assert_eq!(classify(&message, None, &queue), Control::Queue);
            assert_eq!(classify(&message, Some("t0"), &queue), Control::Queue);
            assert_eq!(classify(&message, Some("t1"), &queue), Control::Queue);
        }
    }

    #[test]
    fn a_cancel_finds_its_task_running_queued_or_unknown() {
        let queue = VecDeque::from([queued(Kind::Task, "t2"), queued(Kind::Task, "t3")]);
        let cancel = |task: &str, running: Option<&str>| {
            classify(&queued(Kind::Cancel, task), running, &queue)
        };
        assert_eq!(cancel("t1", Some("t1")), Control::CancelRunning);
        assert_eq!(cancel("t3", Some("t1")), Control::CancelQueued);
        assert_eq!(cancel("t3", None), Control::CancelQueued);
        assert_eq!(cancel("t9", Some("t1")), Control::CancelUnknown);
        assert_eq!(cancel("t1", None), Control::CancelUnknown);
        // A task that is running is not canceled twice from the queue.
        assert_eq!(cancel("t2", Some("t2")), Control::CancelRunning);
    }

    #[test]
    fn outcomes_are_not_taken_by_a_guest() {
        for kind in [
            Kind::Progress,
            Kind::Question,
            Kind::Completed,
            Kind::Failed,
            Kind::Canceled,
        ] {
            assert_eq!(
                classify(&queued(kind, "t1"), Some("t1"), &VecDeque::new()),
                Control::Ignore
            );
        }
    }

    #[test]
    fn a_canceled_reply_carries_the_progress_so_far() {
        assert!(
            canceled_text(Some("Running the tests".to_owned()), true).contains("Running the tests")
        );
        assert!(canceled_text(None, true).starts_with("Canceled"));
        assert!(!canceled_text(None, true).contains("unknown"));
        assert!(canceled_text(None, false).contains("cost of this run is unknown"));
    }
}
