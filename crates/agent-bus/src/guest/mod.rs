//! A guest agent: takes each task from its inbox to Claude Code, in a checkout in the guest, and
//! sends back what Claude does, any question it asks, and the outcome.
//!
//! Tasks run one at a time. A new task forks a primed base session; an answer resumes the Claude
//! session of the task it answers. While a task runs the inbox is still read: tasks and answers
//! wait in a local queue, and a cancel stops the running task (Claude Code and everything it
//! started) or drops a queued one.
//!
//! - `run`: the entry point, the inbox loop and the queue of tasks.
//! - `engine`: which task a message is for, and whether it queues or cancels.
//! - `base`: the primed sessions new tasks fork from, for the prompt cache.
//! - `claude`: the Claude Code command and its process group.
//! - `record`: every `result` event Claude Code prints, kept in the state directory.
//! - `stream`: what Claude Code's output means.

mod base;
mod claude;
mod engine;
mod record;
mod run;
mod stream;

pub use claude::DEFAULT_MODEL;
pub use run::run;
