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
//! - `github`: the GitHub write token, held in memory for `--github` tasks only.
//! - `generation`: the `## Generation` section of a pull request body.
//! - `ledger`: what each task cost and the branch it ended on, for that section.
//! - `pull_request`: the branch a run ended on, and the open pull requests whose section is
//!   rewritten after a `--github` run.
//! - `secrets`: the Claude token, the write token and the bus seed, read from stdin and held in
//!   memory only.
//! - `base`: the primed sessions new tasks fork from, for the prompt cache.
//! - `claude`: the Claude Code command and its process group.
//! - `lock`: one guest agent per name, so no two share a state file.
//! - `record`: every `result` event Claude Code prints, kept in the state directory.
//! - `state`: the guest agent's state file, and what a restart does with it.
//! - `stream`: what Claude Code's output means.

mod base;
mod claude;
mod engine;
mod generation;
mod github;
mod ledger;
mod lock;
mod pull_request;
mod record;
pub mod run;
mod secrets;
mod state;
mod stream;

pub use run::run;
pub use secrets::read_secrets;
