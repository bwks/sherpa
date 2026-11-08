//! What Claude Code's `stream-json` output means: progress, the session, the outcome and what
//! the run used.

use serde::Deserialize;
use serde_json::Value;

use crate::message::{Kind, Usage};

/// A reply line starting with this asks the host agent, and through it the user, a question.
const QUESTION_PREFIX: &str = "QUESTION:";
/// The longest progress message, in characters.
const PROGRESS_CHARS: usize = 300;

/// What a line of Claude Code's `stream-json` output means for the host.
#[derive(Debug, PartialEq)]
pub(super) enum Update {
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

/// The guest log line for a finished run: its totals and cache hit ratio.
pub(super) fn usage_line(task: &str, kind: Kind, usage: &Usage) -> String {
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
pub(super) fn repeats_result(last: &str, result: &str) -> bool {
    last == shorten(result.trim())
}

/// Whether a finished reply is a question, a success or a failure.
pub(super) fn outcome(text: &str, failed: bool) -> Kind {
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
pub(super) fn interpret(line: &str) -> Vec<Update> {
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
