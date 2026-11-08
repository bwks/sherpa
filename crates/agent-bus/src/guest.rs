//! A guest agent: takes each task from its inbox to Claude Code, in a checkout in the guest, and
//! sends back what Claude does, any question it asks, and the outcome.
//!
//! Tasks run one at a time. An answer resumes the Claude session of the task it answers.

use std::collections::HashMap;
use std::env;
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::channel::Channel;
use crate::keys::SEED_VARIABLE;
use crate::message::{Kind, Message, Usage, check_agent_name};

const CLAUDE: &str = "claude";
/// The Claude subscription token, from `claude setup-token`.
const TOKEN_VARIABLE: &str = "CLAUDE_CODE_OAUTH_TOKEN";
/// Removed from Claude Code's environment, so it can only use the subscription.
const API_KEY_VARIABLE: &str = "ANTHROPIC_API_KEY";
/// GitHub tokens. Claude Code only has them for a task that was given GitHub access, so without
/// it `curl` with the token cannot work around the deny list either.
const GITHUB_TOKEN_VARIABLES: [&str; 3] = ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"];
/// Permission checks are skipped: the guest is a disposable lab VM and the agent has root in it.
/// These are still refused. Committing, pushing and GitHub are the user's to decide; the host
/// agent asks them.
const DISALLOWED_TOOLS: &str = "Bash(git commit:*),Bash(git push:*),Bash(git reset:*),Bash(gh:*)";
/// The refused commands for a task given GitHub access: it may commit, push and open pull
/// requests, but merging and destructive operations stay with the user.
///
/// With the token present this list is a guardrail, not a security boundary: it matches command
/// text, and a command written another way (`curl` with the token, a script, an alias) is not
/// caught. The token's own scopes are the boundary.
const GITHUB_DISALLOWED_TOOLS: &str = "Bash(gh pr merge:*),Bash(gh api:*),Bash(gh repo delete:*),\
Bash(gh release:*),Bash(git reset:*),Bash(git push*--force*),Bash(git push* -f),\
Bash(git push* -f *),Bash(git push* --delete*),Bash(git push* -d),Bash(git push* -d *),\
Bash(git push* :*),Bash(git push* +*),Bash(git branch* -D*)";
/// A reply line starting with this asks the host agent, and through it the user, a question.
const QUESTION_PREFIX: &str = "QUESTION:";
/// The longest progress message, in characters.
const PROGRESS_CHARS: usize = 300;
const RULES: &str = "You are a guest agent in a Sherpa lab VM. A host agent sends you tasks over \
a message bus; you do the work in this checkout and report back. Follow AGENTS.md. Do not add \
dependencies. If you need a decision before you can go on, end your reply with one line starting \
with `QUESTION:` and stop; the answer comes as your next message. Otherwise end with a short \
summary of what you did and the results.";
/// Added to the rules for a task without GitHub access.
const RULES_NO_GITHUB: &str = " This task has no GitHub access. Do not commit, push, merge, open \
pull requests or run gh.";
/// Added to the rules for a task with GitHub access.
const RULES_GITHUB: &str = " This task may use GitHub. Do only what the task asks. Commit \
messages end with the Co-Authored-By line the task gives, and have no session links and no \
\"Generated with\" footers. Never merge pull requests or delete anything; the user does that.";

/// What a line of Claude Code's `stream-json` output means for the host.
#[derive(Debug, PartialEq)]
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
        usage: Usage,
    },
}

/// The fields of Claude Code's `result` event that the host needs.
#[derive(Debug, Deserialize)]
struct ResultEvent {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    result: String,
    is_error: Option<bool>,
    #[serde(default)]
    usage: TokenUsage,
    #[serde(default)]
    num_turns: u64,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    duration_api_ms: u64,
    #[serde(default)]
    total_cost_usd: f64,
}

/// The `usage` of a `result` event, under Claude Code's names.
#[derive(Debug, Default, Deserialize)]
struct TokenUsage {
    #[serde(default, rename = "input_tokens")]
    input: u64,
    #[serde(default, rename = "output_tokens")]
    output: u64,
    #[serde(default, rename = "cache_creation_input_tokens")]
    cache_creation: u64,
    #[serde(default, rename = "cache_read_input_tokens")]
    cache_read: u64,
}

/// A Claude session, and whether its task may use GitHub.
#[derive(Debug, Clone, PartialEq)]
struct Session {
    id: String,
    github: bool,
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
                    reply.send(Kind::Failed, format!("{error:#}"), None).await?;
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
    async fn send(&self, kind: Kind, text: String, usage: Option<Usage>) -> Result<()> {
        self.channel
            .send(&Message {
                task: self.task.to_owned(),
                from: self.from.to_owned(),
                to: self.to.to_owned(),
                kind,
                text,
                usage,
                github: false,
            })
            .await
    }
}

/// Run Claude Code on one task or answer, forwarding its progress and its outcome.
async fn work(
    reply: &Reply<'_>,
    message: &Message,
    workdir: &Path,
    sessions: &mut HashMap<String, Session>,
) -> Result<()> {
    let known = sessions.get(&message.task);
    let github = task_github(message, known);
    let mut command = claude_command(workdir, github);
    match (known, message.kind) {
        (Some(session), _) => {
            command.args(["--resume", &session.id]);
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
                    sessions.insert(
                        message.task.clone(),
                        Session {
                            id: session,
                            github,
                        },
                    );
                }
                Update::Text(said) => {
                    if let Some(earlier) = text.replace(said) {
                        reply.send(Kind::Progress, earlier, None).await?;
                    }
                }
                Update::Tool(running) => {
                    if let Some(earlier) = text.take() {
                        reply.send(Kind::Progress, earlier, None).await?;
                    }
                    reply.send(Kind::Progress, running, None).await?;
                }
                Update::Finished {
                    session,
                    text: result,
                    failed,
                    usage,
                } => {
                    sessions.insert(
                        message.task.clone(),
                        Session {
                            id: session,
                            github,
                        },
                    );
                    if let Some(last) = text.take()
                        && !repeats_result(&last, &result)
                    {
                        reply.send(Kind::Progress, last, None).await?;
                    }
                    finished = Some((result, failed, usage));
                }
            }
        }
    }
    let status = child.wait().await.context("Claude Code did not exit")?;
    let (text, failed, usage) =
        finished.with_context(|| format!("Claude Code exited ({status}) without a result"))?;
    let kind = outcome(&text, failed);
    eprintln!("{}", usage_line(&message.task, kind, &usage));
    reply.send(kind, text, Some(usage)).await
}

/// Whether a task may use GitHub: a task says so itself, and an answer has the flag of the task
/// it answers, whatever the answer says.
fn task_github(message: &Message, session: Option<&Session>) -> bool {
    match message.kind {
        Kind::Answer => session.is_some_and(|session| session.github),
        _ => message.github,
    }
}

/// The Claude Code command for a task: its tool limits, its rules and its environment.
fn claude_command(workdir: &Path, github: bool) -> Command {
    let (disallowed, rules) = if github {
        (GITHUB_DISALLOWED_TOOLS, format!("{RULES}{RULES_GITHUB}"))
    } else {
        (DISALLOWED_TOOLS, format!("{RULES}{RULES_NO_GITHUB}"))
    };
    let mut command = Command::new(CLAUDE);
    command
        .args(["-p", "--output-format", "stream-json", "--verbose"])
        .arg("--dangerously-skip-permissions")
        .arg(format!("--disallowedTools={disallowed}"))
        .args(["--append-system-prompt", &rules])
        .current_dir(workdir)
        .env_remove(API_KEY_VARIABLE)
        .env_remove(SEED_VARIABLE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if !github {
        for variable in GITHUB_TOKEN_VARIABLES {
            command.env_remove(variable);
        }
    }
    command
}

/// The guest log line for a finished run: its totals and cache hit ratio.
fn usage_line(task: &str, kind: Kind, usage: &Usage) -> String {
    let ratio = match usage.cache_hit_permille() {
        Some(permille) => format!("{}.{}%", permille / 10, permille % 10),
        None => "n/a".to_owned(),
    };
    format!(
        "Task {task} {kind:?}: input {} output {} cache_creation {} cache_read {} \
         cache_hit {ratio} turns {} duration {}ms api {}ms api_equivalent_cost ${:.4} (not billed)",
        usage.input,
        usage.output,
        usage.cache_creation,
        usage.cache_read,
        usage.num_turns,
        usage.duration_ms,
        usage.duration_api_ms,
        usage.total_cost_usd,
    )
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
        Some("result") => match serde_json::from_value::<ResultEvent>(event) {
            Ok(result) => vec![Update::Finished {
                session: result.session_id,
                text: result.result,
                failed: result.is_error.unwrap_or(true),
                usage: Usage {
                    input: result.usage.input,
                    output: result.usage.output,
                    cache_creation: result.usage.cache_creation,
                    cache_read: result.usage.cache_read,
                    num_turns: result.num_turns,
                    duration_ms: result.duration_ms,
                    duration_api_ms: result.duration_api_ms,
                    total_cost_usd: result.total_cost_usd,
                },
            }],
            Err(error) => {
                eprintln!("Could not parse Claude Code result event: {error}");
                Vec::new()
            }
        },
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

    fn removed(command: &Command) -> Vec<String> {
        command
            .as_std()
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(key, _)| key.to_string_lossy().into_owned())
            .collect()
    }

    fn args(command: &Command) -> Vec<String> {
        command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn task_message(kind: Kind, github: bool) -> Message {
        Message {
            task: "t1".to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind,
            text: String::new(),
            usage: None,
            github,
        }
    }

    #[test]
    fn a_task_without_github_loses_the_tokens_and_keeps_the_full_deny_list() {
        let command = claude_command(Path::new("/work"), false);
        let removed = removed(&command);
        for variable in [
            "ANTHROPIC_API_KEY",
            "AGENT_BUS_SEED",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GH_ENTERPRISE_TOKEN",
        ] {
            assert!(removed.iter().any(|name| name == variable), "{variable}");
        }
        let args = args(&command);
        assert!(args.contains(&format!("--disallowedTools={DISALLOWED_TOOLS}")));
        assert!(args.iter().any(|arg| arg.contains("no GitHub access")));
    }

    #[test]
    fn a_task_with_github_keeps_the_tokens_and_a_narrower_deny_list() {
        let command = claude_command(Path::new("/work"), true);
        let removed = removed(&command);
        for variable in GITHUB_TOKEN_VARIABLES {
            assert!(!removed.iter().any(|name| name == variable), "{variable}");
        }
        for variable in ["ANTHROPIC_API_KEY", "AGENT_BUS_SEED"] {
            assert!(removed.iter().any(|name| name == variable), "{variable}");
        }
        let args = args(&command);
        assert!(args.contains(&format!("--disallowedTools={GITHUB_DISALLOWED_TOOLS}")));
        assert!(args.iter().any(|arg| arg.contains("Co-Authored-By")));
        for refused in [
            "gh pr merge",
            "gh api",
            "gh repo delete",
            "gh release",
            "git reset",
            "--force",
            "-f)",
            "--delete",
            "git push* :*",
            "git branch* -D",
        ] {
            assert!(GITHUB_DISALLOWED_TOOLS.contains(refused), "{refused}");
        }
        for allowed in [
            "Bash(git commit",
            "Bash(git push:*)",
            "Bash(gh:*)",
            "gh pr create",
        ] {
            assert!(!GITHUB_DISALLOWED_TOOLS.contains(allowed), "{allowed}");
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
                usage: Usage::default(),
            }]
        );
    }

    /// The `result` event of a real `claude -p --output-format stream-json --verbose` run (Claude
    /// Code 2.1.319), trimmed to the fields used. That run was not logged in, so its counts are
    /// zero and it is an error; the field names and nesting are as Claude Code writes them.
    const CAPTURED_RESULT: &str = r#"{"duration_api_ms":0,"session_id":"dc1697d1-aa31-4f62-ae89-223eec6ec186","total_cost_usd":0,"usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0},"is_error":true,"num_turns":1,"result":"Not logged in · Please run /login","type":"result","duration_ms":202}"#;

    /// The same event with the counts of a run that used the prompt cache.
    const CACHED_RESULT: &str = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":4200,"duration_api_ms":3900,"num_turns":3,"result":"Done","session_id":"s1","total_cost_usd":0.0421,"usage":{"input_tokens":12,"cache_creation_input_tokens":3000,"cache_read_input_tokens":27000,"output_tokens":850}}"#;

    #[test]
    fn a_captured_result_line_gives_usage() {
        assert_eq!(
            interpret(CAPTURED_RESULT),
            [Update::Finished {
                session: "dc1697d1-aa31-4f62-ae89-223eec6ec186".to_owned(),
                text: "Not logged in · Please run /login".to_owned(),
                failed: true,
                usage: Usage {
                    num_turns: 1,
                    duration_ms: 202,
                    ..Usage::default()
                },
            }]
        );
    }

    #[test]
    fn cache_counts_and_cost_are_read_from_the_result() {
        let [
            Update::Finished {
                usage,
                failed: false,
                ..
            },
        ] = interpret(CACHED_RESULT)[..]
        else {
            panic!("not a finished run");
        };
        assert_eq!(usage.input, 12);
        assert_eq!(usage.output, 850);
        assert_eq!(usage.cache_creation, 3000);
        assert_eq!(usage.cache_read, 27000);
        assert_eq!(usage.num_turns, 3);
        assert_eq!(usage.duration_ms, 4200);
        assert_eq!(usage.duration_api_ms, 3900);
        assert!((usage.total_cost_usd - 0.0421).abs() < 1e-9);
        let line = usage_line("t1", Kind::Completed, &usage);
        assert!(line.contains("cache_hit 89.9%"), "{line}");
        assert!(line.contains("not billed"), "{line}");
    }

    #[test]
    fn a_result_without_usage_still_finishes_the_task() {
        let [Update::Finished { usage, failed, .. }] =
            interpret(r#"{"type":"result","is_error":false,"result":"ok","session_id":"s"}"#)[..]
        else {
            panic!("not a finished run");
        };
        assert!(!failed);
        assert_eq!(usage, Usage::default());
        assert!(usage_line("t", Kind::Completed, &usage).contains("cache_hit n/a"));
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
