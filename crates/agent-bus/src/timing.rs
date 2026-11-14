//! Where an agent's time goes: work, testing, or waiting on the user, with no I/O.
//!
//! Both the guest (from the stream events it reads, stamped as they arrive) and the host (from
//! the timestamps of its imported transcripts) describe a stretch of work as a timeline:
//! prompts that start turns, model responses, and tool calls from their start to their result.
//! `segments` cuts it into buckets:
//!
//! - inside a turn, from its prompt to its last response or tool result, time covered by a
//!   tool call that builds or tests is `Testing`, time covered by an excluded wait (a monitor,
//!   a background wait, `sleep`, a CI watch) is not counted, and the rest is `Work`;
//! - between turns, the time from a turn's end to the user's next prompt is `WaitingOnUser`;
//!   before a prompt Claude Code made itself (a task notification), it is not counted: that is
//!   waiting on background work.
//!
//! A tool call is classified by `classify`, against `TESTING` and `EXCLUDED`, the two fixed
//! lists below and nowhere else. Anything on neither is work.

use std::{iter, mem};

use serde::Serialize;
use serde_json::Value;

/// Bash commands that build or test, as the words a command starts with. A command matches
/// when any of its parts does: parts are split at `;`, `&`, `|`, parentheses and line ends
/// outside quotes, and each part's leading variable assignments and wrappers (`WRAPPERS`) are
/// skipped. A command run through `ssh` or `sh -c` is matched too.
const TESTING: &[&[&str]] = &[
    &["cargo", "build"],
    &["cargo", "clippy"],
    &["cargo", "test"],
    &["cargo", "nextest"],
    &["sherpa", "up"],
    &["sherpa", "down"],
    &["sherpa", "destroy"],
    &["sherpa", "redeploy"],
    &["sherpa", "resume"],
];
/// A command part that runs a script under this directory is testing too.
const TESTING_SCRIPTS: &str = "tests/labs/";
/// Bash commands that only wait, as the words a part starts with and a flag it must have: not
/// counted.
const EXCLUDED: &[(&[&str], Option<&str>)] = &[
    (&["sleep"], None),
    (&["gh", "run", "watch"], None),
    (&["gh", "pr", "checks"], Some("--watch")),
];
/// Tools that only wait for something else: not counted.
const EXCLUDED_TOOLS: &[&str] = &["Monitor", "TaskOutput", "BashOutput"];
/// Words in front of a command that run it: skipped before matching. `timeout` also takes its
/// options and duration.
const WRAPPERS: &[&str] = &["sudo", "time", "env", "exec", "nice", "command"];
/// Shells that run a script given as their first argument, or a command after `-c`.
const SHELLS: &[&str] = &["bash", "sh"];
/// The tool that runs shell commands.
const BASH_TOOL: &str = "Bash";
/// Characters that end one part of a shell command, outside quotes.
const PART_ENDS: &[char] = &[';', '&', '|', '(', ')', '\n', '`'];
/// How deep commands within commands (`ssh host 'sh -c ...'`) are looked into.
const MAX_NESTING: usize = 3;

/// What a tool call is, for time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Work,
    Testing,
    /// A wait that is not counted.
    Excluded,
}

/// The buckets time is recorded in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bucket {
    Work,
    Testing,
    WaitingOnUser,
}

/// A prompt, which starts a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prompt {
    pub at_ms: u64,
    /// Whether the user wrote it, rather than Claude Code (a task notification).
    pub from_user: bool,
}

/// One model response: when it started being written, when it was last written, and the work
/// item it is charged to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Response {
    pub first_ms: u64,
    pub last_ms: u64,
    pub item: Option<i64>,
}

/// One tool call, from its start to its result; `end_ms` is `None` until the result is seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call {
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub class: Class,
}

/// A stretch of one session or run, in any order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timeline {
    pub prompts: Vec<Prompt>,
    pub responses: Vec<Response>,
    pub calls: Vec<Call>,
}

/// A stretch of time in one bucket, charged to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub bucket: Bucket,
    pub item: Option<i64>,
}

/// Time in each bucket, in milliseconds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Times {
    pub work: u64,
    pub testing: u64,
    pub waiting: u64,
}

/// What a guest saw in Claude Code's stream that tells where time went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tick {
    /// The model wrote a response.
    Responded,
    /// A tool call started.
    Called { id: String, class: Class },
    /// A tool call returned.
    Returned { id: String },
    /// Claude Code gave the model a prompt of its own, such as a task notification.
    Prompted,
}

/// One run's timeline, as the guest reads Claude Code's stream: each event stamped when it was
/// read.
#[derive(Debug, Clone, Default)]
pub struct RunClock {
    timeline: Timeline,
    /// The ID of each call in `timeline.calls`, in the same order.
    ids: Vec<String>,
}

/// A turn of a timeline: from its prompt to its last activity.
#[derive(Debug, Clone, Copy)]
struct Turn {
    start_ms: u64,
    end_ms: u64,
    from_user: bool,
}

impl Times {
    /// Add `segment` to its bucket.
    pub fn add(&mut self, segment: &Segment) {
        let ms = segment.end_ms.saturating_sub(segment.start_ms);
        let bucket = match segment.bucket {
            Bucket::Work => &mut self.work,
            Bucket::Testing => &mut self.testing,
            Bucket::WaitingOnUser => &mut self.waiting,
        };
        *bucket = bucket.saturating_add(ms);
    }
}

impl RunClock {
    /// A run that started at `at_ms`: the clock starts when work starts, not when the task was
    /// sent.
    pub fn start(at_ms: u64) -> Self {
        Self {
            timeline: Timeline {
                prompts: vec![Prompt {
                    at_ms,
                    from_user: false,
                }],
                ..Timeline::default()
            },
            ids: Vec::new(),
        }
    }

    /// Note `tick`, read at `at_ms`.
    pub fn observe(&mut self, tick: Tick, at_ms: u64) {
        let timeline = &mut self.timeline;
        match tick {
            Tick::Responded => timeline.responses.push(Response {
                first_ms: at_ms,
                last_ms: at_ms,
                item: None,
            }),
            Tick::Called { id, class } => {
                timeline.calls.push(Call {
                    start_ms: at_ms,
                    end_ms: None,
                    class,
                });
                self.ids.push(id);
            }
            Tick::Returned { id } => {
                if let Some(index) = self.ids.iter().position(|called| *called == id)
                    && let Some(call) = timeline.calls.get_mut(index)
                {
                    call.end_ms.get_or_insert(at_ms);
                }
            }
            Tick::Prompted => timeline.prompts.push(Prompt {
                at_ms,
                from_user: false,
            }),
        }
    }

    /// The run's work and testing time when it finished at `at_ms`.
    pub fn times(&self, at_ms: u64) -> Times {
        let mut timeline = self.timeline.clone();
        timeline.responses.push(Response {
            first_ms: at_ms,
            last_ms: at_ms,
            item: None,
        });
        total(&segments(&timeline))
    }
}

/// What the tool `name` called with `input` is, for time: testing, an excluded wait, or work.
pub fn classify(name: &str, input: &Value) -> Class {
    if EXCLUDED_TOOLS.contains(&name) {
        return Class::Excluded;
    }
    if name != BASH_TOOL {
        return Class::Work;
    }
    let Some(command) = input["command"].as_str() else {
        return Class::Work;
    };
    classify_command(command)
}

/// What a Bash `command` is: testing when any part builds or tests, else an excluded wait when
/// any part only waits, else work.
pub fn classify_command(command: &str) -> Class {
    let mut parts = Vec::new();
    command_parts(command, 0, &mut parts);
    if parts.iter().any(|words| is_testing(words)) {
        Class::Testing
    } else if parts.iter().any(|words| {
        EXCLUDED.iter().any(|(pattern, flag)| {
            starts_with(words, pattern)
                && flag.is_none_or(|flag| words.iter().any(|word| word == flag))
        })
    }) {
        Class::Excluded
    } else {
        Class::Work
    }
}

/// Add the parts of `command`, each as its words from its command on, to `parts`; with the
/// parts of the commands it runs through `ssh` or `sh -c`, `depth` levels down.
fn command_parts(command: &str, depth: usize, parts: &mut Vec<Vec<String>>) {
    for words in split(command) {
        let words = command_words(&words);
        let inner = match words.as_slice() {
            [ssh, rest @ ..] if ssh == "ssh" => {
                // Options, then the host, then the command.
                let options = rest.iter().take_while(|word| word.starts_with('-')).count();
                rest.get(options + 1..).map(|command| command.join(" "))
            }
            [shell, flag, command, ..] if SHELLS.contains(&shell.as_str()) && flag == "-c" => {
                Some(command.clone())
            }
            _ => None,
        };
        if let Some(inner) = inner
            && depth < MAX_NESTING
        {
            command_parts(&inner, depth + 1, parts);
        }
        if !words.is_empty() {
            parts.push(words);
        }
    }
}

/// The parts of `command`, split at `PART_ENDS` outside quotes, as their words: a quoted
/// string is one word, without its quotes.
fn split(command: &str) -> Vec<Vec<String>> {
    let mut parts = Vec::new();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut in_word = false;
    for character in command.chars() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => word.push(character),
            (None, '\'' | '"') => {
                quote = Some(character);
                in_word = true;
            }
            (None, _) if PART_ENDS.contains(&character) || character.is_whitespace() => {
                if in_word {
                    words.push(mem::take(&mut word));
                    in_word = false;
                }
                if PART_ENDS.contains(&character) && !words.is_empty() {
                    parts.push(mem::take(&mut words));
                }
            }
            (None, _) => {
                word.push(character);
                in_word = true;
            }
        }
    }
    if in_word {
        words.push(word);
    }
    if !words.is_empty() {
        parts.push(words);
    }
    parts
}

/// The words of one command part from its command on: variable assignments and wrappers in
/// front are skipped.
fn command_words(words: &[String]) -> Vec<String> {
    let mut rest = words;
    loop {
        match rest {
            [first, tail @ ..] if is_assignment(first) || WRAPPERS.contains(&first.as_str()) => {
                rest = tail;
            }
            [timeout, tail @ ..] if timeout == "timeout" => {
                // `timeout` takes options, some with a value, then its duration.
                let duration = tail.iter().position(|word| {
                    word.chars()
                        .next()
                        .is_some_and(|first| first.is_ascii_digit())
                });
                rest = duration
                    .and_then(|at| tail.get(at + 1..))
                    .unwrap_or_default();
            }
            _ => return rest.to_vec(),
        }
    }
}

/// Whether `word` sets a variable, such as `RUST_LOG=debug`.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

/// Whether the command `words` builds or tests.
fn is_testing(words: &[String]) -> bool {
    let script = match words {
        [shell, script, ..] if SHELLS.contains(&shell.as_str()) => script,
        [program, ..] => program,
        [] => return false,
    };
    if script.contains(TESTING_SCRIPTS) {
        return true;
    }
    // `cargo +nightly test` names a toolchain first.
    let words: Vec<String> = match words {
        [cargo, toolchain, rest @ ..] if cargo == "cargo" && toolchain.starts_with('+') => {
            iter::once(cargo.clone())
                .chain(rest.iter().cloned())
                .collect()
        }
        _ => words.to_vec(),
    };
    TESTING.iter().any(|pattern| starts_with(&words, pattern))
}

/// Whether `words` start with `pattern`.
fn starts_with(words: &[String], pattern: &[&str]) -> bool {
    words.len() >= pattern.len() && words.iter().zip(pattern).all(|(word, part)| word == part)
}

/// `timeline` cut into segments of work, testing and waiting on the user, oldest first. Time
/// before the first prompt, waits that are not counted, and the gaps before prompts Claude Code
/// made are left out.
pub fn segments(timeline: &Timeline) -> Vec<Segment> {
    let mut responses = timeline.responses.clone();
    responses.sort_by_key(|response| response.first_ms);
    let turns = turns(timeline);
    let mut found = Vec::new();
    for (index, turn) in turns.iter().enumerate() {
        if index > 0 && turn.from_user {
            let previous = turns[index - 1].end_ms;
            if turn.start_ms > previous {
                found.push(Segment {
                    start_ms: previous,
                    end_ms: turn.start_ms,
                    bucket: Bucket::WaitingOnUser,
                    item: item_at(&responses, previous, previous),
                });
            }
        }
        let calls: Vec<(u64, u64, Class)> = timeline
            .calls
            .iter()
            .filter(|call| call.start_ms >= turn.start_ms && call.start_ms <= turn.end_ms)
            .map(|call| {
                let end = call.end_ms.unwrap_or(turn.end_ms).min(turn.end_ms);
                (call.start_ms, end.max(call.start_ms), call.class)
            })
            .collect();
        let mut bounds = vec![turn.start_ms, turn.end_ms];
        for (start, end, _) in &calls {
            bounds.push(*start);
            bounds.push(*end);
        }
        bounds.sort_unstable();
        bounds.dedup();
        for pair in bounds.windows(2) {
            let (start, end) = (pair[0], pair[1]);
            let covering = calls
                .iter()
                .filter(|(from, to, _)| *from <= start && *to >= end)
                .map(|(_, _, class)| *class);
            let mut class = Class::Work;
            for covered in covering {
                class = match (class, covered) {
                    (Class::Testing, _) | (_, Class::Testing) => Class::Testing,
                    (Class::Excluded, _) | (_, Class::Excluded) => Class::Excluded,
                    (Class::Work, Class::Work) => Class::Work,
                };
            }
            let bucket = match class {
                Class::Work => Bucket::Work,
                Class::Testing => Bucket::Testing,
                Class::Excluded => continue,
            };
            push_segment(
                &mut found,
                Segment {
                    start_ms: start,
                    end_ms: end,
                    bucket,
                    item: item_at(&responses, start, turn.end_ms),
                },
            );
        }
    }
    found
}

/// Add `segment` to `found`, joined to the last one when it carries on from it.
fn push_segment(found: &mut Vec<Segment>, segment: Segment) {
    if let Some(last) = found.last_mut()
        && last.end_ms == segment.start_ms
        && last.bucket == segment.bucket
        && last.item == segment.item
    {
        last.end_ms = segment.end_ms;
        return;
    }
    found.push(segment);
}

/// The turns of `timeline`, oldest first: each from its prompt to the last response or tool
/// result before the next prompt; a turn with neither ends where it starts.
fn turns(timeline: &Timeline) -> Vec<Turn> {
    let mut prompts = timeline.prompts.clone();
    prompts.sort_by_key(|prompt| prompt.at_ms);
    prompts
        .iter()
        .enumerate()
        .map(|(index, prompt)| {
            let next = prompts.get(index + 1).map_or(u64::MAX, |next| next.at_ms);
            let within = |at: u64| at >= prompt.at_ms && at < next;
            let responses = timeline
                .responses
                .iter()
                .filter(|response| within(response.first_ms))
                .map(|response| response.last_ms.min(next));
            let calls = timeline
                .calls
                .iter()
                .filter(|call| within(call.start_ms))
                .filter_map(|call| call.end_ms.map(|end| end.min(next)));
            let end_ms = responses.chain(calls).fold(prompt.at_ms, u64::max);
            Turn {
                start_ms: prompt.at_ms,
                end_ms,
                from_user: prompt.from_user,
            }
        })
        .collect()
}

/// The item of the latest response started at or before `at_ms`, else of the first one started
/// by `until_ms`: the response that made the call, or the one the model was writing. `responses`
/// is in order.
fn item_at(responses: &[Response], at_ms: u64, until_ms: u64) -> Option<i64> {
    responses
        .iter()
        .rev()
        .find(|response| response.first_ms <= at_ms)
        .or_else(|| {
            responses
                .iter()
                .find(|response| response.first_ms > at_ms && response.first_ms <= until_ms)
        })
        .and_then(|response| response.item)
}

/// The time of `segments` in each bucket.
pub fn total(segments: &[Segment]) -> Times {
    let mut times = Times::default();
    for segment in segments {
        times.add(segment);
    }
    times
}

/// A duration as `1h 07m`, `12m 30s` or `45s`.
#[cfg(feature = "record")]
pub fn duration(ms: u64) -> String {
    let seconds = ms / 1000;
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

/// The time between each question of a task and the answer after it, summed: the time it
/// waited on the user. `messages` are the task's questions (`true`) and answers (`false`) with
/// their times, oldest first. A question with no answer yet counts nothing.
#[cfg(feature = "record")]
pub fn waiting_on_answers(messages: &[(bool, u64)]) -> u64 {
    let mut waiting: u64 = 0;
    let mut asked = None;
    for (question, at_ms) in messages {
        match (question, asked) {
            (true, None) => asked = Some(*at_ms),
            (false, Some(since)) => {
                waiting = waiting.saturating_add(at_ms.saturating_sub(since));
                asked = None;
            }
            _ => {}
        }
    }
    waiting
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn bash(command: &str) -> Class {
        classify("Bash", &json!({ "command": command }))
    }

    fn user(at_ms: u64) -> Prompt {
        Prompt {
            at_ms,
            from_user: true,
        }
    }

    fn response(first_ms: u64, last_ms: u64) -> Response {
        Response {
            first_ms,
            last_ms,
            item: None,
        }
    }

    fn call(start_ms: u64, end_ms: u64, class: Class) -> Call {
        Call {
            start_ms,
            end_ms: Some(end_ms),
            class,
        }
    }

    #[test]
    fn builds_tests_and_lab_commands_are_testing() {
        for command in [
            "cargo build --workspace",
            "cargo clippy --workspace --all-targets -- -D warnings",
            "cargo test -p agent-bus",
            "cargo nextest run",
            "cargo +nightly test",
            "cd crates/agent-bus && cargo test",
            "RUST_LOG=debug cargo test 2>&1 | tail -5",
            "timeout 600 cargo test",
            "timeout -s KILL 600 cargo build",
            "sudo cargo test",
            "ssh dev 'cd sherpa && cargo test --workspace'",
            "(cd x; cargo build)",
            "./tests/labs/basic.sh",
            "bash tests/labs/run.sh linux",
            "sherpa up",
            "sherpa destroy --yes",
            "sherpa redeploy r1",
            "cargo fmt && cargo clippy --workspace",
        ] {
            assert_eq!(bash(command), Class::Testing, "{command}");
        }
    }

    #[test]
    fn waits_are_excluded_and_beat_nothing_but_testing() {
        for command in [
            "sleep 60",
            "sleep 5 && git status",
            "gh run watch 123",
            "gh pr checks 12 --watch",
            "GH_TOKEN=x gh pr checks --watch",
        ] {
            assert_eq!(bash(command), Class::Excluded, "{command}");
        }
        assert_eq!(bash("sleep 5; cargo test"), Class::Testing);
        for tool in ["Monitor", "TaskOutput", "BashOutput"] {
            assert_eq!(classify(tool, &json!({})), Class::Excluded, "{tool}");
        }
    }

    #[test]
    fn everything_else_is_work() {
        for command in [
            "git status",
            "cargo fmt",
            "cat tests/labs/basic.sh",
            "grep -rn 'cargo test' docs",
            "echo sleep",
            "gh pr checks 12",
            "gh run view 123",
            "ls tests",
            "",
        ] {
            assert_eq!(bash(command), Class::Work, "{command}");
        }
        for (tool, input) in [
            ("Read", json!({ "file_path": "/x" })),
            ("Edit", json!({})),
            ("Agent", json!({ "prompt": "cargo test" })),
            ("Bash", json!({})),
        ] {
            assert_eq!(classify(tool, &input), Class::Work, "{tool}");
        }
    }

    #[test]
    fn a_turn_with_each_kind_of_tool_call_is_cut_into_buckets() {
        // Prompt at 0; the model writes until 10, reads (10-12), tests (15-75), sleeps
        // (80-140), runs a monitor (145-200) and answers at 210.
        let timeline = Timeline {
            prompts: vec![user(0)],
            responses: vec![
                response(5, 10),
                response(13, 15),
                response(76, 80),
                response(141, 145),
                response(201, 210),
            ],
            calls: vec![
                call(10, 12, Class::Work),
                call(15, 75, Class::Testing),
                call(80, 140, Class::Excluded),
                call(145, 200, Class::Excluded),
            ],
        };
        let times = total(&segments(&timeline));
        assert_eq!(
            times,
            Times {
                // 0-15, 75-80, 140-145, 200-210.
                work: 15 + 5 + 5 + 10,
                testing: 60,
                waiting: 0,
            }
        );
    }

    #[test]
    fn parallel_calls_count_once_and_testing_wins() {
        let timeline = Timeline {
            prompts: vec![user(0)],
            responses: vec![response(0, 10), response(100, 110)],
            calls: vec![
                call(10, 60, Class::Work),
                call(20, 90, Class::Testing),
                call(30, 100, Class::Excluded),
            ],
        };
        // 0-20 work, 20-90 testing, 90-100 excluded, 100-110 work.
        assert_eq!(
            total(&segments(&timeline)),
            Times {
                work: 30,
                testing: 70,
                waiting: 0,
            }
        );
    }

    #[test]
    fn the_gap_before_a_users_prompt_is_waiting_and_before_a_notification_is_not() {
        let timeline = Timeline {
            prompts: vec![
                user(0),
                Prompt {
                    at_ms: 1_000,
                    from_user: false,
                },
                user(5_000),
            ],
            responses: vec![
                response(0, 100),
                response(1_000, 1_200),
                response(5_000, 5_050),
            ],
            calls: vec![],
        };
        assert_eq!(
            total(&segments(&timeline)),
            Times {
                work: 100 + 200 + 50,
                testing: 0,
                // 1,200 to 5,000; the 100 to 1,000 before the notification is not counted.
                waiting: 3_800,
            }
        );
    }

    #[test]
    fn time_before_the_first_prompt_and_a_call_without_a_result_stay_in_the_turn() {
        let timeline = Timeline {
            prompts: vec![user(100)],
            responses: vec![response(50, 60), response(100, 120), response(180, 200)],
            calls: vec![Call {
                start_ms: 120,
                end_ms: None,
                class: Class::Testing,
            }],
        };
        // 100-120 work, 120-200 testing (to the turn's end).
        assert_eq!(
            total(&segments(&timeline)),
            Times {
                work: 20,
                testing: 80,
                waiting: 0,
            }
        );
    }

    #[test]
    fn segments_take_the_item_of_the_response_before_them() {
        let timeline = Timeline {
            prompts: vec![user(0), user(500)],
            responses: vec![
                Response {
                    first_ms: 10,
                    last_ms: 20,
                    item: Some(1),
                },
                Response {
                    first_ms: 50,
                    last_ms: 100,
                    item: Some(2),
                },
                Response {
                    first_ms: 500,
                    last_ms: 600,
                    item: None,
                },
            ],
            calls: vec![call(20, 50, Class::Testing)],
        };
        let found = segments(&timeline);
        let by_item = |item: Option<i64>| {
            total(
                &found
                    .iter()
                    .filter(|segment| segment.item == item)
                    .copied()
                    .collect::<Vec<_>>(),
            )
        };
        // 0-20 work and 20-50 testing go to item 1 (the first response, then the one that
        // made the call); 50-100 work and the wait 100-500 to item 2; the last turn to none.
        assert_eq!(
            by_item(Some(1)),
            Times {
                work: 20,
                testing: 30,
                waiting: 0,
            }
        );
        assert_eq!(
            by_item(Some(2)),
            Times {
                work: 50,
                testing: 0,
                waiting: 400,
            }
        );
        assert_eq!(by_item(None).work, 100);
    }

    #[test]
    fn a_runs_clock_starts_with_the_run_and_leaves_out_its_waits() {
        // The task was queued before 1,000: the clock starts when the run does.
        let mut clock = RunClock::start(1_000);
        clock.observe(Tick::Responded, 3_000);
        clock.observe(
            Tick::Called {
                id: "t1".to_owned(),
                class: Class::Testing,
            },
            3_000,
        );
        clock.observe(
            Tick::Called {
                id: "t2".to_owned(),
                class: Class::Excluded,
            },
            3_000,
        );
        clock.observe(
            Tick::Returned {
                id: "t2".to_owned(),
            },
            3_010,
        );
        clock.observe(
            Tick::Returned {
                id: "t1".to_owned(),
            },
            63_000,
        );
        clock.observe(Tick::Responded, 64_000);
        // The model ends its turn to wait on a background task; the notification comes later.
        clock.observe(
            Tick::Called {
                id: "t3".to_owned(),
                class: Class::Excluded,
            },
            64_000,
        );
        clock.observe(
            Tick::Returned {
                id: "t3".to_owned(),
            },
            124_000,
        );
        clock.observe(Tick::Prompted, 300_000);
        clock.observe(Tick::Responded, 301_000);
        assert_eq!(
            clock.times(302_000),
            Times {
                // 1,000-3,000, 63,000-64,000, 300,000-302,000.
                work: 2_000 + 1_000 + 2_000,
                testing: 60_000,
                waiting: 0,
            }
        );
    }

    #[cfg(feature = "record")]
    #[test]
    fn a_question_waits_until_its_answer() {
        assert_eq!(waiting_on_answers(&[]), 0);
        assert_eq!(
            waiting_on_answers(&[(true, 1_000), (false, 61_000), (true, 70_000)]),
            60_000
        );
        assert_eq!(
            waiting_on_answers(&[
                (true, 1_000),
                (false, 2_000),
                (true, 3_000),
                (false, 10_000)
            ]),
            8_000
        );
        // An answer without a question counts nothing.
        assert_eq!(waiting_on_answers(&[(false, 5)]), 0);
    }

    #[cfg(feature = "record")]
    #[test]
    fn durations_print_in_hours_and_minutes_or_minutes_and_seconds() {
        assert_eq!(duration(0), "0s");
        assert_eq!(duration(45_999), "45s");
        assert_eq!(duration(750_000), "12m 30s");
        assert_eq!(duration(4_020_000), "1h 07m");
        assert_eq!(duration(36_000_000), "10h 00m");
    }
}
