//! A guest agent: takes each task from its inbox to Claude Code, in a checkout in the guest, and
//! sends back what Claude does, any question it asks, and the outcome.
//!
//! Tasks run one at a time. An answer resumes the Claude session of the task it answers.

use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::channel::Channel;
use crate::message::{Kind, Message, check_agent_name};

const CLAUDE: &str = "claude";
/// The Claude subscription token, from `claude setup-token`.
const TOKEN_VARIABLE: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// Removed from Claude Code's environment, so it can only use the subscription.
const API_KEY_VARIABLE: &str = "ANTHROPIC_API_KEY";
const ALLOWED_TOOLS: &str = "Read,Edit,Write,Glob,Grep,Bash";
/// Committing, pushing and GitHub are the user's to decide; the host agent asks them.
const DISALLOWED_TOOLS: &str = "Bash(git commit:*),Bash(git push:*),Bash(git reset:*),Bash(gh:*)";
/// A reply line starting with this asks the host agent, and through it the user, a question.
const QUESTION_PREFIX: &str = "QUESTION:";
/// The longest progress message, in characters.
const PROGRESS_CHARS: usize = 300;
const RULES: &str = "You are a guest agent in a Sherpa lab VM. A host agent sends you tasks over \
a message bus; you do the work in this checkout and report back. Follow AGENTS.md. Do not commit, \
push, merge, open pull requests or run gh, and do not add dependencies. If you need a decision \
before you can go on, end your reply with one line starting with `QUESTION:` and stop; the answer \
comes as your next message. Otherwise end with a short summary of what you did and the results.";

/// What a line of Claude Code's `stream-json` output means for the host.
#[derive(Debug, PartialEq, Eq)]
enum Update {
    /// The Claude session the task runs in.
    Session(String),
    /// What Claude says. Its last text is also its result.
    Text(String),
    /// A tool Claude runs, and on what.
    Tool(String),
    Finished {
        session: String,
        text: String,
        failed: bool,
    },
}

/// Where replies about one task go.
struct Reply<'a> {
    channel: &'a Channel,
    task: &'a str,
    from: &'a str,
    to: &'a str,
}

/// Take tasks from `name`'s inbox until the bus closes, and reply to `reply_to`.
///
/// # Errors
///
/// Fails when a name is invalid, the subscription token is not set, or the bus fails. A task
/// that fails is reported to `reply_to` instead.
pub async fn run(channel: &Channel, name: &str, reply_to: &str, workdir: &Path) -> Result<()> {
    check_agent_name(name)?;
    check_agent_name(reply_to)?;
    ensure!(
        env::var_os(TOKEN_VARIABLE).is_some(),
        "Set {TOKEN_VARIABLE}: guest agents use the Claude subscription, never API billing"
    );
    let mut inbox = channel.inbox(name).await?;
    let mut sessions = HashMap::new();
    eprintln!("Guest agent '{name}' is waiting for tasks");
    while let Some(delivery) = inbox.next().await? {
        // Acknowledged at once: a task can run far longer than the bus waits for an
        // acknowledgement, and it must not be delivered twice.
        delivery.ack().await?;
        let message = delivery.message;
        match message.kind {
            Kind::Task | Kind::Answer => {
                eprintln!(
                    "{:?} {} from '{}'",
                    message.kind, message.task, message.from
                );
                let reply = Reply {
                    channel,
                    task: &message.task,
                    from: name,
                    to: reply_to,
                };
                if let Err(error) = work(&reply, &message, workdir, &mut sessions).await {
                    reply.send(Kind::Failed, format!("{error:#}")).await?;
                }
            }
            Kind::Progress | Kind::Question | Kind::Completed | Kind::Failed => {
                eprintln!(
                    "Ignoring a {:?} message from '{}': guest agents take tasks and answers",
                    message.kind, message.from
                );
            }
        }
    }
    Ok(())
}

impl Reply<'_> {
    async fn send(&self, kind: Kind, text: String) -> Result<()> {
        self.channel
            .send(&Message {
                task: self.task.to_owned(),
                from: self.from.to_owned(),
                to: self.to.to_owned(),
                kind,
                text,
            })
            .await
    }
}

/// Run Claude Code on one task or answer, forwarding its progress and its outcome.
async fn work(
    reply: &Reply<'_>,
    message: &Message,
    workdir: &Path,
    sessions: &mut HashMap<String, String>,
) -> Result<()> {
    let mut command = Command::new(CLAUDE);
    command
        .args(["-p", "--output-format", "stream-json", "--verbose"])
        .args(["--permission-mode", "acceptEdits"])
        .arg(format!("--allowedTools={ALLOWED_TOOLS}"))
        .arg(format!("--disallowedTools={DISALLOWED_TOOLS}"))
        .args(["--append-system-prompt", RULES])
        .current_dir(workdir)
        .env_remove(API_KEY_VARIABLE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    match (sessions.get(&message.task), message.kind) {
        (Some(session), _) => {
            command.args(["--resume", session]);
        }
        (None, Kind::Answer) => {
            bail!(
                "No session for task {}: an answer must follow a question",
                message.task
            )
        }
        (None, _) => {}
    }
    let mut child = command.spawn().context("Unable to start Claude Code")?;
    // The prompt goes in on stdin, so the tool lists cannot swallow it as an argument.
    let mut stdin = child.stdin.take().context("Claude Code has no input")?;
    stdin
        .write_all(message.text.as_bytes())
        .await
        .context("Unable to give Claude Code the task")?;
    drop(stdin);
    let stdout = child.stdout.take().context("Claude Code has no output")?;
    let mut lines = BufReader::new(stdout).lines();
    let mut finished = None;
    // Claude's last text is also its result, so each text waits for the next update to show
    // whether it was the last.
    let mut text = None;
    while let Some(line) = lines
        .next_line()
        .await
        .context("Unable to read Claude Code's output")?
    {
        for update in interpret(&line) {
            match update {
                Update::Session(session) => {
                    sessions.insert(message.task.clone(), session);
                }
                Update::Text(said) => {
                    if let Some(earlier) = text.replace(said) {
                        reply.send(Kind::Progress, earlier).await?;
                    }
                }
                Update::Tool(running) => {
                    if let Some(earlier) = text.take() {
                        reply.send(Kind::Progress, earlier).await?;
                    }
                    reply.send(Kind::Progress, running).await?;
                }
                Update::Finished {
                    session,
                    text: result,
                    failed,
                } => {
                    sessions.insert(message.task.clone(), session);
                    if let Some(last) = text.take()
                        && !repeats_result(&last, &result)
                    {
                        reply.send(Kind::Progress, last).await?;
                    }
                    finished = Some((result, failed));
                }
            }
        }
    }
    let status = child.wait().await.context("Claude Code did not exit")?;
    let (text, failed) =
        finished.with_context(|| format!("Claude Code exited ({status}) without a result"))?;
    reply.send(outcome(&text, failed), text).await
}

/// Whether Claude's last text is its result again, as progress shortens it.
fn repeats_result(last: &str, result: &str) -> bool {
    last == shorten(result.trim())
}

/// Whether a finished reply is a question, a success or a failure.
fn outcome(text: &str, failed: bool) -> Kind {
    if failed {
        Kind::Failed
    } else if text
        .lines()
        .any(|line| line.trim_start().starts_with(QUESTION_PREFIX))
    {
        Kind::Question
    } else {
        Kind::Completed
    }
}

/// What one line of Claude Code's `stream-json` output means: lines that are not events, and
/// events the host does not need, mean nothing.
fn interpret(line: &str) -> Vec<Update> {
    let Ok(event) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let session = event["session_id"].as_str().unwrap_or_default().to_owned();
    match event["type"].as_str() {
        Some("system") if !session.is_empty() => vec![Update::Session(session)],
        Some("assistant") => event["message"]["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(progress)
            .collect(),
        Some("result") => vec![Update::Finished {
            session,
            text: event["result"].as_str().unwrap_or_default().to_owned(),
            failed: event["is_error"].as_bool().unwrap_or(true),
        }],
        _ => Vec::new(),
    }
}

/// One block of an assistant message as progress: its text, or the tool it runs and on what.
fn progress(block: &Value) -> Option<Update> {
    match block["type"].as_str()? {
        "text" => {
            let said = block["text"].as_str()?.trim();
            if said.is_empty() {
                return None;
            }
            Some(Update::Text(shorten(said)))
        }
        "tool_use" => {
            let input = &block["input"];
            let target = ["command", "file_path", "pattern", "path"]
                .into_iter()
                .find_map(|key| input[key].as_str())
                .unwrap_or_default();
            Some(Update::Tool(shorten(&format!(
                "{}: {target}",
                block["name"].as_str()?
            ))))
        }
        _ => None,
    }
}

fn shorten(text: &str) -> String {
    if text.chars().count() <= PROGRESS_CHARS {
        return text.to_owned();
    }
    let start: String = text.chars().take(PROGRESS_CHARS).collect();
    format!("{start}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_events_become_session_progress_and_outcome() {
        assert_eq!(
            interpret(r#"{"type":"system","subtype":"init","session_id":"s1"}"#),
            [Update::Session("s1".to_owned())]
        );
        assert_eq!(
            interpret(
                r#"{"type":"assistant","session_id":"s1","message":{"content":[
                    {"type":"text","text":" Running the tests. "},
                    {"type":"tool_use","name":"Bash","input":{"command":"cargo test -p db"}},
                    {"type":"tool_use","name":"Read","input":{"file_path":"/src/lib.rs"}}]}}"#
            ),
            [
                Update::Text("Running the tests.".to_owned()),
                Update::Tool("Bash: cargo test -p db".to_owned()),
                Update::Tool("Read: /src/lib.rs".to_owned()),
            ]
        );
        assert_eq!(
            interpret(
                r#"{"type":"result","is_error":false,"result":"All passed","session_id":"s1"}"#
            ),
            [Update::Finished {
                session: "s1".to_owned(),
                text: "All passed".to_owned(),
                failed: false,
            }]
        );
    }

    #[test]
    fn other_lines_mean_nothing() {
        for line in ["", "not json", r#"{"type":"user","message":{}}"#] {
            assert_eq!(interpret(line), [], "{line}");
        }
    }

    #[test]
    fn the_last_text_is_dropped_only_when_it_repeats_the_result() {
        assert!(repeats_result("All 9 passed.", "All 9 passed.\n"));
        assert!(!repeats_result("Running the tests.", "All 9 passed."));
        let long = "x".repeat(PROGRESS_CHARS + 10);
        assert!(repeats_result(&shorten(&long), &long));
    }

    #[test]
    fn a_question_line_makes_the_outcome_a_question() {
        assert_eq!(outcome("Done.\nAll tests pass.", false), Kind::Completed);
        assert_eq!(
            outcome("Found two options.\nQUESTION: add the crate?", false),
            Kind::Question
        );
        assert_eq!(outcome("QUESTION: anything", true), Kind::Failed);
    }

    #[test]
    fn long_progress_is_shortened() {
        assert_eq!(shorten("short"), "short");
        let long = "é".repeat(PROGRESS_CHARS + 5);
        assert_eq!(shorten(&long).chars().count(), PROGRESS_CHARS + 1);
    }
}
