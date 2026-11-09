//! A guest agent: takes each task from its inbox to Claude Code, in a checkout in the guest, and
//! sends back what Claude does, any question it asks, and the outcome.
//!
//! Tasks run one at a time. A new task forks a primed base session; an answer resumes the Claude
//! session of the task it answers. While a task runs the inbox is still read: tasks and answers
//! wait in a local queue, and a cancel stops the running task (Claude Code and everything it
//! started) or drops a queued one. What it knows (sessions, the queue, the running task, the
//! bases) is kept in a state file, so a restart goes on where it stopped.
//!
//! - `run`: the entry point, the inbox loop and the queue of tasks.
//! - `engine`: which task a message is for, and whether it queues or cancels.
//! - `base`: the primed sessions new tasks fork from, for the prompt cache.
//! - `claude`: the Claude Code command and its process group.
//! - `lock`: one guest agent per name, so no two share a state file.
//! - `record`: every `result` event Claude Code prints, kept in the state directory.
//! - `state`: the guest agent's state file, and what a restart does with it.
//! - `stream`: what Claude Code's output means.

mod base;
mod claude;
mod engine;
mod lock;
mod record;
pub mod run;
mod state;
mod stream;

pub use run::run;
