//! What Claude Code's `stream-json` output means: progress, the session, the outcome and what
//! the run used.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::message::{Kind, ModelUsage, TaskUsage, Usage, cache_hit_permille};

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
    /// A run's `result` event. Its `usage.duration_api_ms` is still the session's total so far:
    /// the guest takes the previous run's off (`api_time_of_run`).
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
    /// Per model, for this run only. (`modelUsage` is the session's total.)
    #[serde(default)]
    model_usage_since_last_result: BTreeMap<String, ModelUsage>,
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

/// The guest log line for a finished run: the run, then the task so far.
pub(super) fn usage_line(task: &str, kind: Kind, run: &Usage, so_far: &TaskUsage) -> String {
    let ratio = |permille: Option<u64>| match permille {
        Some(permille) => format!("{}.{}%", permille / 10, permille % 10),
        None => "n/a".to_owned(),
    };
    format!(
        "Task {task} {kind:?}: run: input {} output {} cache_creation {} cache_read {}          cache_hit {} turns {} duration {}ms api {}ms api_equivalent_cost ${:.4} (not billed);          session_cost ${:.4}; task so far: {} runs input {} output {} cache_creation {}          cache_read {} cache_hit {} turns {} duration {}ms api {}ms api_equivalent_cost ${:.4}",
        run.input,
        run.output,
        run.cache_creation,
        run.cache_read,
        ratio(cache_hit_permille(
            run.input,
            run.cache_creation,
            run.cache_read
        )),
        run.num_turns,
        run.duration_ms,
        run.duration_api_ms,
        run.cost_usd,
        run.session_cost_usd,
        so_far.runs,
        so_far.input,
        so_far.output,
        so_far.cache_creation,
        so_far.cache_read,
        ratio(cache_hit_permille(
            so_far.input,
            so_far.cache_creation,
            so_far.cache_read
        )),
        so_far.num_turns,
        so_far.duration_ms,
        so_far.duration_api_ms,
        so_far.cost_usd,
    )
}

/// The guest log line for a priming run: the run that builds a base session. Its cost is counted
/// in the task that triggered it, which is named here.
pub(super) fn priming_line(task: &str, github: bool, session: &str, run: &Usage) -> String {
    let variant = if github { "GitHub" } else { "no-GitHub" };
    format!(
        "Task {task}: priming run for the {variant} base session {session}, counted in the task: \
         input {} output {} cache_creation {} cache_read {} turns {} api_equivalent_cost ${:.4} \
         (not billed)",
        run.input, run.output, run.cache_creation, run.cache_read, run.num_turns, run.cost_usd,
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
            Ok(result) => {
                let models: Vec<ModelUsage> = result
                    .model_usage_since_last_result
                    .into_iter()
                    .map(|(model, usage)| ModelUsage { model, ..usage })
                    .collect();
                vec![Update::Finished {
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
                        cost_usd: models.iter().map(|model| model.cost_usd).sum(),
                        session_cost_usd: result.total_cost_usd,
                        models,
                    },
                }]
            }
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

    use crate::message::api_time_of_run;

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

    /// The `result` events of the three runs of task 562a9215 (Claude Code 2.1.319, one session
    /// resumed twice), trimmed to the fields used. `total_cost_usd` and `duration_api_ms` are the
    /// session's totals so far; `usage` and `model_usage_since_last_result` are the run's own.
    const RUN_1: &str = r#"{"duration_api_ms":1556,"session_id":"2cc3d453-311c-4686-b082-56f0c9569e34","total_cost_usd":0.0386093,"usage":{"input_tokens":2,"cache_creation_input_tokens":9281,"cache_read_input_tokens":13213,"output_tokens":16},"is_error":false,"num_turns":1,"result":"QUESTION: Which colour do you like?","type":"result","duration_ms":1994,"model_usage_since_last_result":{"claude-sonnet-5-5":{"inputTokens":2,"outputTokens":16,"cacheReadInputTokens":13213,"cacheCreationInputTokens":9281,"costUSD":0.0386093,"canonicalModel":"claude-sonnet-5-5","costBasis":"list"}}}"#;

    const RUN_2: &str = r#"{"duration_api_ms":2937,"session_id":"2cc3d453-311c-4686-b082-56f0c9569e34","total_cost_usd":0.0557514,"usage":{"input_tokens":4,"cache_creation_input_tokens":3777,"cache_read_input_tokens":18761,"output_tokens":15},"is_error":false,"num_turns":1,"result":"QUESTION: Which number do you like?","type":"result","duration_ms":1748,"model_usage_since_last_result":{"claude-sonnet-5-5":{"inputTokens":4,"outputTokens":15,"cacheReadInputTokens":18761,"cacheCreationInputTokens":3777,"costUSD":0.0171421,"canonicalModel":"claude-sonnet-5-5","costBasis":"list"}}}"#;

    const RUN_3: &str = r#"{"duration_api_ms":4218,"session_id":"2cc3d453-311c-4686-b082-56f0c9569e34","total_cost_usd":0.0582012,"usage":{"input_tokens":4,"cache_creation_input_tokens":37,"cache_read_input_tokens":22538,"output_tokens":4},"is_error":false,"num_turns":1,"result":"done.","type":"result","duration_ms":1695,"model_usage_since_last_result":{"claude-sonnet-5-5":{"inputTokens":4,"outputTokens":4,"cacheReadInputTokens":22538,"cacheCreationInputTokens":37,"costUSD":0.0024498,"canonicalModel":"claude-sonnet-5-5","costBasis":"list"}}}"#;

    fn finished(line: &str) -> Usage {
        let updates = interpret(line);
        let [Update::Finished { usage, .. }] = &updates[..] else {
            panic!("not a finished run");
        };
        usage.clone()
    }

    #[test]
    fn a_runs_cost_is_the_sum_of_its_models_and_not_the_session_total() {
        let first = finished(RUN_1);
        assert_eq!(first.input, 2);
        assert_eq!(first.output, 16);
        assert_eq!(first.cache_creation, 9281);
        assert_eq!(first.cache_read, 13213);
        assert_eq!(first.num_turns, 1);
        assert_eq!(first.duration_ms, 1994);
        assert!((first.cost_usd - 0.038_609_3).abs() < 1e-9);
        assert_eq!(first.models.len(), 1);
        assert_eq!(first.models[0].model, "claude-sonnet-5-5");
        assert_eq!(first.models[0].canonical_model, "claude-sonnet-5-5");
        assert_eq!(first.models[0].cost_basis, "list");
        let second = finished(RUN_2);
        assert!((second.cost_usd - 0.017_142_1).abs() < 1e-9);
        assert!((second.session_cost_usd - 0.055_751_4).abs() < 1e-9);
        assert_eq!(second.cache_read, 18761);
        // The event's API time is the session's so far.
        assert_eq!(second.duration_api_ms, 2937);
    }

    #[test]
    fn the_three_runs_add_up_to_the_sessions_cost() {
        let mut task = TaskUsage::default();
        let mut previous = None;
        let mut api = Vec::new();
        for line in [RUN_1, RUN_2, RUN_3] {
            let mut run = finished(line);
            run.duration_api_ms = api_time_of_run(run.duration_api_ms, previous);
            previous = Some(finished(line).duration_api_ms);
            api.push(run.duration_api_ms);
            task.add(&run);
        }
        assert_eq!(task.runs, 3);
        assert!((task.cost_usd - 0.058_201_2).abs() < 1e-9);
        assert_eq!(format!("{:.4}", task.cost_usd), "0.0582");
        // It equals the session total the last event reports.
        assert!((task.cost_usd - finished(RUN_3).session_cost_usd).abs() < 1e-9);
        assert_eq!(api, [1556, 1381, 1281]);
        assert_eq!(task.duration_api_ms, 4218);
        assert_eq!(task.input, 10);
        assert_eq!(task.output, 35);
        assert_eq!(task.cache_creation, 13095);
        assert_eq!(task.cache_read, 54512);
        assert_eq!(task.num_turns, 3);
        assert_eq!(task.duration_ms, 1994 + 1748 + 1695);
    }

    #[test]
    fn the_usage_line_shows_the_run_and_the_task() {
        let run = finished(RUN_2);
        let mut task = TaskUsage::default();
        task.add(&finished(RUN_1));
        task.add(&run);
        let line = usage_line("562a9215", Kind::Question, &run, &task);
        assert!(line.contains("run: input 4 output 15"), "{line}");
        assert!(line.contains("session_cost $0.0558"), "{line}");
        assert!(
            line.contains("task so far: 2 runs input 6 output 31"),
            "{line}"
        );
        assert!(line.contains("not billed"), "{line}");
    }

    #[test]
    fn a_result_with_no_models_costs_nothing() {
        let usage = finished(
            r#"{"type":"result","is_error":true,"result":"Not logged in","session_id":"s","total_cost_usd":0,"duration_api_ms":0,"num_turns":1,"duration_ms":202,"usage":{"input_tokens":0,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":0}}"#,
        );
        assert_eq!(usage.models, []);
        assert!(usage.cost_usd.abs() < f64::EPSILON);
        assert_eq!(usage.duration_ms, 202);
    }

    #[test]
    fn a_result_without_usage_still_finishes_the_task() {
        let updates =
            interpret(r#"{"type":"result","is_error":false,"result":"ok","session_id":"s"}"#);
        let [Update::Finished { usage, failed, .. }] = &updates[..] else {
            panic!("not a finished run");
        };
        assert!(!failed);
        assert_eq!(*usage, Usage::default());
        assert!(
            usage_line("t", Kind::Completed, usage, &TaskUsage::default())
                .contains("cache_hit n/a")
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

    #[test]
    fn the_priming_line_is_marked_and_names_the_task_it_is_counted_in() {
        let run = Usage {
            cache_creation: 9148,
            cost_usd: 0.038,
            ..Usage::default()
        };
        let line = priming_line("t1", false, "base-1", &run);
        assert!(line.contains("priming run"), "{line}");
        assert!(line.contains("no-GitHub base session base-1"), "{line}");
        assert!(line.contains("Task t1"), "{line}");
        assert!(line.contains("cache_creation 9148"), "{line}");
        assert!(priming_line("t1", true, "b", &run).contains("GitHub base"));
    }
}
