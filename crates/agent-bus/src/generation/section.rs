//! The `## Generation` section of a pull request body: what the agent tasks and the host spent
//! on the work item behind the pull request, kept by the host between two markers. Pure text and
//! data; `sync` supplies the rows and reads and writes the bodies.
//!
//! The section carries its rows twice: as a markdown table for people, and as TOML in an HTML
//! comment, which is what is read back. The data has a format version. Rewriting the section
//! keeps the rows of tasks the record does not know (written before the record, by guests),
//! replaces the rows of tasks it knows, whose figures are cumulative, drops the rows of tasks it
//! has in another item, and replaces the host's rows. The table is never parsed, and nothing outside the markers is touched.
//!
//! Guests wrote format 1, with no version and no host rows, and read it with
//! `deny_unknown_fields`: a guest still running an older agent-bus refuses a host-written
//! section, so it cannot undo one.

use std::collections::BTreeSet;
use std::ops::Range;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::message::{TaskUsage, cache_hit_permille, check_agent_name};
use crate::report::usage::{HOST, Time};
use crate::timing::duration;

/// The line that opens the section.
pub const START: &str = "<!-- agent-bus:generation -->";
/// The line that closes the section.
pub const END: &str = "<!-- /agent-bus:generation -->";
/// The format of the data this agent-bus writes: 2 added the version, the host's rows and the
/// time of each row.
const FORMAT: u32 = 2;
/// Opens the HTML comment holding the rows as TOML.
const DATA_START: &str = "<!-- agent-bus:generation-data";
/// Closes an HTML comment.
const COMMENT_END: &str = "-->";
const HEADING: &str = "## Generation";
/// The title of the section, as a hand-written heading outside the markers would have it.
const TITLE: &str = "generation";
/// What a hand-written table row of work outside the agent bus says.
const NOT_MEASURED: &str = "not measured";
const NOTE: &str = "Figures from agent-bus, at list price and not billed: the agents run on the \
Claude subscription. Guest tasks cost what Claude Code reports (`costUSD`); the host's own work \
is priced from its Claude Code transcripts. Host cost and time are included from the first host \
import; earlier host work is not measured. Time is work, testing (builds, tests and labs) and \
waiting on the user.";
const TABLE_HEAD: &str = "| Task | Agent | Models | Runs | Cost (USD) | Cache hit | Input | \
Output | Cache read | Cache write | Work | Testing | Waiting on user |\n\
|---|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|";
/// What a time cell shows when the time is not known.
const UNKNOWN: &str = "n/a";
/// After a total that leaves out what is not known: unpriced responses, or rows without time.
const INCOMPLETE: &str = " (incomplete)";
/// The longest task ID, model name or model list kept.
const MAX_ID: usize = 64;
const MAX_MODELS: usize = 16;

/// An open fenced code block, as in `CommonMark`: three or more backticks or tildes after at
/// most three spaces. Holds its character and how many times it repeats.
struct Fence {
    mark: char,
    len: usize,
}

/// One agent task in the section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub task: String,
    pub agent: String,
    /// Every model its runs used, subagents' included, sorted.
    pub models: Vec<String>,
    /// Its totals over all its runs.
    pub usage: TaskUsage,
    /// Its work, testing and waiting on the user; unknown in rows written before format 2.
    #[serde(default, skip_serializing_if = "Time::is_unknown")]
    pub time: Time,
}

/// The host's own usage on one model, its subagents' included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostRow {
    pub model: String,
    /// Its cost, the responses with a price only.
    pub usage: TaskUsage,
    pub time: Time,
    /// How many of its responses have no price, whose cost is not in `usage`; `None` when all
    /// have one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unpriced: Option<u64>,
}

/// Every row of a section: the agent tasks, then the host's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rows {
    pub tasks: Vec<Row>,
    pub host: Vec<HostRow>,
    /// Every task the record has in a work item, this one's or another's: a row of one of them
    /// that this item no longer has is dropped, not carried. Not written in the data.
    pub known: BTreeSet<String>,
}

/// The TOML in the data comment.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    /// Absent in format 1.
    #[serde(default = "first_format")]
    version: u32,
    #[serde(default)]
    task: Vec<Row>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    host: Vec<HostRow>,
}

/// The format of data with no version: what guests wrote.
fn first_format() -> u32 {
    1
}

/// `body` with its section rewritten to hold `ours` and the rows of other tasks it already had,
/// or with the section appended when it has none. Everything outside the markers is unchanged.
///
/// # Errors
///
/// Fails when the markers are not one start followed by one end, the data in the section cannot
/// be read, or a row has a task, agent or model that cannot be shown safely.
pub fn update_body(body: &str, ours: &Rows) -> Result<String> {
    check_rows(ours)?;
    let Some(range) = section_range(body)? else {
        let section = render(ours)?;
        let kept = body.trim_end();
        return if kept.is_empty() {
            Ok(format!("{section}\n"))
        } else {
            Ok(format!("{kept}\n\n{section}\n"))
        };
    };
    let theirs = read(&body[range.clone()])?;
    let section = render(&merge(theirs.tasks, ours))?;
    Ok(format!(
        "{}{section}{}",
        &body[..range.start],
        &body[range.end..]
    ))
}

/// The section of `body`, from its start marker to its end marker; `None` when it has none.
///
/// # Errors
///
/// Fails when the markers are not one start followed by one end.
pub fn section_of(body: &str) -> Result<Option<&str>> {
    Ok(section_range(body)?.map(|range| &body[range]))
}

/// The rows in the data comment of `section`; none when it has no data comment. Data of format
/// 1 is read as it is: its rows have no time.
///
/// # Errors
///
/// Fails when the comment is not closed, its data does not parse or is of a newer format, or a
/// row cannot be shown safely.
pub fn read(section: &str) -> Result<Rows> {
    let Some(at) = section.find(DATA_START) else {
        return Ok(Rows::default());
    };
    let data = &section[at + DATA_START.len()..];
    let end = data
        .find(COMMENT_END)
        .context("The Generation data comment in the pull request body is not closed")?;
    let data: Data = toml::from_str(&data[..end])
        .context("Unable to read the Generation data in the pull request body")?;
    ensure!(
        data.version <= FORMAT,
        "The Generation data in the pull request body is format {}, newer than {FORMAT} that \
         this agent-bus writes: use a newer agent-bus",
        data.version
    );
    let rows = Rows {
        tasks: data.task,
        host: data.host,
        known: BTreeSet::new(),
    };
    check_rows(&rows)?;
    Ok(rows)
}

/// Whether `body` has, outside the section and outside code blocks, a `Generation` heading or a
/// table row saying work was not measured: a section written by hand, which is left for the
/// user to tidy.
///
/// # Errors
///
/// Fails when the markers are not one start followed by one end.
pub fn hand_written(body: &str) -> Result<bool> {
    let section = section_range(body)?;
    Ok(outside_fences(body).into_iter().any(|(at, line)| {
        let text = line.trim();
        let inside = section.as_ref().is_some_and(|range| range.contains(&at));
        let heading =
            text.starts_with('#') && text.trim_start_matches('#').trim().to_lowercase() == TITLE;
        let row = text.starts_with('|') && text.to_lowercase().contains(NOT_MEASURED);
        !inside && (heading || row)
    }))
}

/// Each line of `body` that is not in a fenced code block, nor a fence itself, with where it
/// starts.
fn outside_fences(body: &str) -> Vec<(usize, &str)> {
    let mut lines = Vec::new();
    let mut fence: Option<Fence> = None;
    let mut offset = 0;
    for line in body.split_inclusive('\n') {
        let at = offset;
        offset += line.len();
        if let Some(open) = &fence {
            if open.is_closed_by(line) {
                fence = None;
            }
            continue;
        }
        if let Some(open) = Fence::opened_by(line) {
            fence = Some(open);
            continue;
        }
        lines.push((at, line));
    }
    lines
}

/// Where the section is in `body`, from the start of its first marker to the end of its last;
/// `None` when it has no markers. A marker counts only when it is a line of its own, apart from
/// surrounding whitespace, and is not inside a fenced code block, so a description that quotes
/// the markers is not taken for the section.
fn section_range(body: &str) -> Result<Option<Range<usize>>> {
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    for (at, line) in outside_fences(body) {
        let text = line.trim();
        let text_at = at + (line.len() - line.trim_start().len());
        if text == START {
            starts.push(text_at);
        } else if text == END {
            ends.push(text_at);
        }
    }
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([start], [end]) if start < end => Ok(Some(*start..end + END.len())),
        _ => bail!(
            "The pull request body has its Generation markers out of order or more than once: \
             {START} must be followed by {END}, once each, each on a line of its own. Fix the \
             body by hand"
        ),
    }
}

impl Fence {
    /// The fence `line` opens, if it opens one.
    fn opened_by(line: &str) -> Option<Self> {
        let (mark, len, rest) = fence_run(line)?;
        // A backtick fence's info string cannot hold a backtick: "```a```" is inline code.
        if mark == '`' && rest.contains('`') {
            return None;
        }
        Some(Self { mark, len })
    }

    /// Whether `line` closes this fence: a run of the same character at least as long, with
    /// nothing but whitespace after it.
    fn is_closed_by(&self, line: &str) -> bool {
        fence_run(line).is_some_and(|(mark, len, rest)| {
            mark == self.mark && len >= self.len && rest.trim().is_empty()
        })
    }
}

/// The fence character of `line`, how many times it repeats, and the rest of the line, when
/// the line starts, after at most three spaces, with three or more backticks or tildes.
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let text = line.trim_start_matches(' ');
    if line.len() - text.len() > 3 {
        return None;
    }
    let mark = text.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = text.len() - text.trim_start_matches(mark).len();
    (len >= 3).then(|| (mark, len, &text[len..]))
}

/// `theirs` in their order, each replaced by the row of `ours` for the same task, then the rest
/// of `ours`' tasks in their order, and `ours`' host rows. A row of `theirs` that `ours` does
/// not have is kept only when the record does not know its task (`ours.known`): a task the
/// record has in another item has left this one.
fn merge(theirs: Vec<Row>, ours: &Rows) -> Rows {
    let mut tasks: Vec<Row> = theirs
        .into_iter()
        .filter(|row| {
            !ours.known.contains(&row.task) || ours.tasks.iter().any(|our| our.task == row.task)
        })
        .map(|row| {
            ours.tasks
                .iter()
                .find(|our| our.task == row.task)
                .cloned()
                .unwrap_or(row)
        })
        .collect();
    for row in &ours.tasks {
        if !tasks.iter().any(|kept| kept.task == row.task) {
            tasks.push(row.clone());
        }
    }
    Rows {
        tasks,
        host: ours.host.clone(),
        known: BTreeSet::new(),
    }
}

/// The section, from its start marker to its end marker.
fn render(rows: &Rows) -> Result<String> {
    let mut total = TaskUsage::default();
    let mut times: Vec<Time> = Vec::new();
    let mut lines = vec![
        START.to_owned(),
        HEADING.to_owned(),
        String::new(),
        NOTE.to_owned(),
        String::new(),
        TABLE_HEAD.to_owned(),
    ];
    for row in &rows.tasks {
        add(&mut total, &row.usage);
        total.runs = total.runs.saturating_add(row.usage.runs);
        times.push(row.time);
        lines.push(format!(
            "| {} | {} | {} | {} | {} | {} |",
            row.task,
            row.agent,
            row.models.join(", "),
            grouped(row.usage.runs),
            figures(&row.usage).join(" | "),
            clock(&row.time).join(" | ")
        ));
    }
    for row in &rows.host {
        add(&mut total, &row.usage);
        times.push(row.time);
        let mut cells = figures(&row.usage);
        if let Some(unpriced) = row.unpriced {
            cells[0] = format!("{} + {unpriced} unpriced", cells[0]);
        }
        lines.push(format!(
            "| {HOST} | {HOST} | {} | | {} | {} |",
            row.model,
            cells.join(" | "),
            clock(&row.time).join(" | ")
        ));
    }
    let mut cost_cells = figures(&total);
    if rows.host.iter().any(|row| row.unpriced.is_some()) {
        cost_cells[0].push_str(INCOMPLETE);
    }
    let mut time_cells = clock(&sum(&times));
    for (cell, missing) in time_cells.iter_mut().zip(missing(&times)) {
        if missing && cell != UNKNOWN {
            cell.push_str(INCOMPLETE);
        }
    }
    lines.push(format!(
        "| **Total** | | | **{}** | {} | {} |",
        grouped(total.runs),
        bold(cost_cells).join(" | "),
        bold(time_cells).join(" | ")
    ));
    lines.push(String::new());
    // Every string in the rows is checked to hold no `-->` (`check_rows`), so the data cannot
    // close the comment early.
    let data = toml::to_string(&Data {
        version: FORMAT,
        task: rows.tasks.clone(),
        host: rows.host.clone(),
    })
    .context("Unable to write the Generation data")?;
    lines.push(format!("{DATA_START}\n{data}{COMMENT_END}"));
    lines.push(END.to_owned());
    Ok(lines.join("\n"))
}

/// `cells`, each in bold.
fn bold(cells: Vec<String>) -> Vec<String> {
    cells
        .into_iter()
        .map(|cell| format!("**{cell}**"))
        .collect()
}

/// Add the cost and tokens of `usage` to `total`.
fn add(total: &mut TaskUsage, usage: &TaskUsage) {
    total.cost_usd += usage.cost_usd;
    total.input = total.input.saturating_add(usage.input);
    total.output = total.output.saturating_add(usage.output);
    total.cache_creation = total.cache_creation.saturating_add(usage.cache_creation);
    total.cache_read = total.cache_read.saturating_add(usage.cache_read);
}

/// Each bucket of `times` added up over the rows that know it; unknown when none does.
fn sum(times: &[Time]) -> Time {
    let mut total = Time::default();
    for time in times {
        for (sum, part) in [
            (&mut total.work, time.work),
            (&mut total.testing, time.testing),
            (&mut total.waiting, time.waiting),
        ] {
            if let Some(part) = part {
                *sum = Some(sum.unwrap_or_default().saturating_add(part));
            }
        }
    }
    total
}

/// For each time bucket (work, testing, waiting on the user), whether a row of `times` does not
/// know it: then the total is a lower bound.
fn missing(times: &[Time]) -> [bool; 3] {
    [
        times.iter().any(|time| time.work.is_none()),
        times.iter().any(|time| time.testing.is_none()),
        times.iter().any(|time| time.waiting.is_none()),
    ]
}

/// The cost and token cells of a row: cost, cache hit, input, output, cache read, cache write.
fn figures(usage: &TaskUsage) -> Vec<String> {
    let hit = match cache_hit_permille(usage.input, usage.cache_creation, usage.cache_read) {
        Some(permille) => format!("{}.{}%", permille / 10, permille % 10),
        None => "n/a".to_owned(),
    };
    vec![
        format!("${:.4}", usage.cost_usd),
        hit,
        grouped(usage.input),
        grouped(usage.output),
        grouped(usage.cache_read),
        grouped(usage.cache_creation),
    ]
}

/// The time cells of a row: work, testing, waiting on the user.
fn clock(time: &Time) -> Vec<String> {
    [time.work, time.testing, time.waiting]
        .into_iter()
        .map(|ms| ms.map_or_else(|| UNKNOWN.to_owned(), duration))
        .collect()
}

/// `number` with a comma between each group of three digits.
fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Whether `text` is short and of letters, digits and `-._`, and of `extra`.
fn is_id(text: &str, extra: &[char]) -> bool {
    !text.is_empty()
        && text.len() <= MAX_ID
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c) || extra.contains(&c))
}

/// Refuse a row whose task, agent or models could break the table or the comment: each must be
/// short and of letters, digits and `-._`, models also `:/@[]`.
fn check_rows(rows: &Rows) -> Result<()> {
    for row in &rows.tasks {
        ensure!(
            is_id(&row.task, &[]),
            "The task ID '{}' cannot go in a Generation section",
            row.task.escape_debug()
        );
        check_agent_name(&row.agent)?;
        ensure!(
            row.models.len() <= MAX_MODELS,
            "Task {} has more than {MAX_MODELS} models",
            row.task
        );
        for model in &row.models {
            check_model(model)?;
        }
    }
    for row in &rows.host {
        check_model(&row.model)?;
    }
    Ok(())
}

/// Refuse a model name that could break the table or the comment.
fn check_model(model: &str) -> Result<()> {
    ensure!(
        is_id(model, &[':', '/', '@', '[', ']']),
        "The model '{}' cannot go in a Generation section",
        model.escape_debug()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    const MINUTE: u64 = 60_000;

    fn row(task: &str, agent: &str, models: &[&str], usage: TaskUsage) -> Row {
        Row {
            task: task.to_owned(),
            agent: agent.to_owned(),
            models: models.iter().map(|model| (*model).to_owned()).collect(),
            usage,
            time: Time::default(),
        }
    }

    fn tasks(rows: &[Row]) -> Rows {
        Rows {
            tasks: rows.to_vec(),
            ..Rows::default()
        }
    }

    fn time(work: u64, testing: u64, waiting: u64) -> Time {
        Time {
            work: Some(work),
            testing: Some(testing),
            waiting: Some(waiting),
        }
    }

    fn first() -> Row {
        row(
            "3f9a1c2e",
            "dev01",
            &["claude-haiku-5-5", "claude-opus-5-5"],
            TaskUsage {
                runs: 3,
                cost_usd: 4.1873,
                input: 1204,
                output: 48_310,
                cache_creation: 371_902,
                cache_read: 6_120_455,
                ..TaskUsage::default()
            },
        )
    }

    fn second() -> Row {
        row(
            "7b20d4aa",
            "dev02",
            &["claude-sonnet-5-5"],
            TaskUsage {
                runs: 1,
                cost_usd: 0.612,
                input: 88,
                output: 9412,
                cache_creation: 126_004,
                cache_read: 1_402_330,
                ..TaskUsage::default()
            },
        )
    }

    fn host(model: &str, cost_usd: f64, time: Time) -> HostRow {
        HostRow {
            model: model.to_owned(),
            usage: TaskUsage {
                cost_usd,
                input: 100,
                output: 2000,
                cache_creation: 10_000,
                cache_read: 90_000,
                ..TaskUsage::default()
            },
            time,
            unpriced: None,
        }
    }

    /// The table part of a section: its lines that start with `|`.
    fn table(body: &str) -> Vec<&str> {
        body.lines().filter(|line| line.starts_with('|')).collect()
    }

    /// The data comment of a section written in format 1, as guests wrote it.
    fn format_1(rows: &[Row]) -> String {
        let mut data = String::new();
        for row in rows {
            let models: Vec<String> = row
                .models
                .iter()
                .map(|model| format!("\"{model}\""))
                .collect();
            write!(
                data,
                "[[task]]\ntask = \"{}\"\nagent = \"{}\"\nmodels = [{}]\n\n[task.usage]\nruns = {}\n\
                 cost_usd = {}\ninput = {}\noutput = {}\ncache_creation = {}\ncache_read = {}\n\
                 num_turns = 0\nduration_ms = 0\nduration_api_ms = 0\n\n",
                row.task,
                row.agent,
                models.join(", "),
                row.usage.runs,
                row.usage.cost_usd,
                row.usage.input,
                row.usage.output,
                row.usage.cache_creation,
                row.usage.cache_read
            )
            .unwrap();
        }
        format!(
            "{START}\n## Generation\n\nFigures from agent-bus.\n\n| Task |\n|---|\n\n\
             {DATA_START}\n{data}{COMMENT_END}\n{END}"
        )
    }

    #[test]
    fn two_tasks_render_as_a_table_with_a_total() {
        let body = update_body("", &tasks(&[first(), second()])).unwrap();
        assert!(
            body.starts_with(&format!("{START}\n## Generation\n\n")),
            "{body}"
        );
        assert!(body.contains("list price"), "{body}");
        assert!(body.contains("not billed"), "{body}");
        assert_eq!(
            table(&body),
            [
                "| Task | Agent | Models | Runs | Cost (USD) | Cache hit | Input | Output | Cache read | Cache write | Work | Testing | Waiting on user |",
                "|---|---|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|",
                "| 3f9a1c2e | dev01 | claude-haiku-5-5, claude-opus-5-5 | 3 | $4.1873 | 94.2% | 1,204 | 48,310 | 6,120,455 | 371,902 | n/a | n/a | n/a |",
                "| 7b20d4aa | dev02 | claude-sonnet-5-5 | 1 | $0.6120 | 91.7% | 88 | 9,412 | 1,402,330 | 126,004 | n/a | n/a | n/a |",
                "| **Total** | | | **4** | **$4.7993** | **93.7%** | **1,292** | **57,722** | **7,522,785** | **497,906** | **n/a** | **n/a** | **n/a** |",
            ]
        );
        assert!(body.trim_end().ends_with(END), "{body}");
    }

    #[test]
    fn host_rows_and_time_columns_render_and_the_total_adds_them_up() {
        let mut timed = first();
        timed.time = time(67 * MINUTE, 12 * MINUTE + 30_000, 45_000);
        // A task from an older guest: its runs reported no work or testing time.
        let mut older = second();
        older.time = Time {
            waiting: Some(MINUTE),
            ..Time::default()
        };
        let rows = Rows {
            tasks: vec![timed, older],
            host: vec![
                host("claude-haiku-5-5", 0.25, time(0, 0, 0)),
                host(
                    "claude-opus-5-5",
                    1.5,
                    time(90 * MINUTE, 5 * MINUTE, 30 * MINUTE),
                ),
            ],
            ..Rows::default()
        };
        let body = update_body("", &rows).unwrap();
        assert_eq!(
            table(&body)[2..],
            [
                "| 3f9a1c2e | dev01 | claude-haiku-5-5, claude-opus-5-5 | 3 | $4.1873 | 94.2% | 1,204 | 48,310 | 6,120,455 | 371,902 | 1h 07m | 12m 30s | 45s |",
                "| 7b20d4aa | dev02 | claude-sonnet-5-5 | 1 | $0.6120 | 91.7% | 88 | 9,412 | 1,402,330 | 126,004 | n/a | n/a | 1m 00s |",
                "| host | host | claude-haiku-5-5 | | $0.2500 | 89.9% | 100 | 2,000 | 90,000 | 10,000 | 0s | 0s | 0s |",
                "| host | host | claude-opus-5-5 | | $1.5000 | 89.9% | 100 | 2,000 | 90,000 | 10,000 | 1h 30m | 5m 00s | 30m 00s |",
                "| **Total** | | | **4** | **$6.5493** | **93.6%** | **1,492** | **61,722** | **7,702,785** | **517,906** | **2h 37m (incomplete)** | **17m 30s (incomplete)** | **31m 45s** |",
            ]
        );
        assert!(body.contains("first host import"), "{body}");
        assert!(body.contains("earlier host work is not measured"), "{body}");
        // The data reads back as the rows, with the format version first.
        let range = section_range(&body).unwrap().unwrap();
        assert_eq!(read(&body[range]).unwrap(), rows);
        assert!(
            body.contains(&format!("{DATA_START}\nversion = 2\n")),
            "{body}"
        );
    }

    #[test]
    fn the_data_comment_reads_back_as_the_rows() {
        let body = update_body("", &tasks(&[first(), second()])).unwrap();
        let range = section_range(&body).unwrap().unwrap();
        assert_eq!(read(&body[range]).unwrap(), tasks(&[first(), second()]));
    }

    #[test]
    fn a_body_without_the_section_gets_it_appended_after_a_blank_line() {
        let body = update_body(
            "Fixes the thing.\n\n## Tests\n\nAll pass.\n\n",
            &tasks(&[first()]),
        )
        .unwrap();
        assert!(
            body.starts_with(&format!(
                "Fixes the thing.\n\n## Tests\n\nAll pass.\n\n{START}\n"
            )),
            "{body}"
        );
        assert!(body.ends_with(&format!("{END}\n")), "{body}");
    }

    #[test]
    fn only_the_section_is_rewritten() {
        let before = "Intro\r\nwith CRLF\r\n\r\n";
        let after = "\r\n\r\n## Notes\r\nHand-written rows: host work, not measured.\r\n";
        let old = update_body("", &tasks(&[first()])).unwrap();
        let body = format!("{before}{}{after}", old.trim_end());
        let mut changed = first();
        changed.usage.runs = 4;
        let updated = update_body(&body, &tasks(&[changed])).unwrap();
        assert!(
            updated.starts_with(&format!("{before}{START}")),
            "{updated}"
        );
        assert!(updated.ends_with(&format!("{END}{after}")), "{updated}");
        assert!(table(&updated).iter().any(|line| {
            line.starts_with("| 3f9a1c2e | dev01 | claude-haiku-5-5, claude-opus-5-5 | 4 |")
        }));
    }

    #[test]
    fn updating_twice_gives_the_same_body() {
        let once = update_body("Body\n", &tasks(&[first(), second()])).unwrap();
        assert_eq!(
            update_body(&once, &tasks(&[first(), second()])).unwrap(),
            once
        );
        assert_eq!(update_body(&once, &tasks(&[second()])).unwrap(), once);
    }

    #[test]
    fn rows_of_other_tasks_are_kept_and_ours_replace_theirs() {
        let theirs = update_body("", &tasks(&[first(), second()])).unwrap();
        let mut ours = second();
        ours.usage.runs = 2;
        let third = row(
            "c0ffee00",
            "dev01",
            &["claude-opus-5-5"],
            TaskUsage::default(),
        );
        let body = update_body(&theirs, &tasks(&[third.clone(), ours.clone()])).unwrap();
        let range = section_range(&body).unwrap().unwrap();
        assert_eq!(read(&body[range]).unwrap(), tasks(&[first(), ours, third]));
    }

    #[test]
    fn the_hosts_rows_are_replaced_whole() {
        let old = Rows {
            tasks: vec![first()],
            host: vec![
                host("claude-haiku-5-5", 0.1, time(1, 1, 1)),
                host("claude-opus-5-5", 1.0, time(1, 1, 1)),
            ],
            ..Rows::default()
        };
        let body = update_body("", &old).unwrap();
        let new = Rows {
            tasks: Vec::new(),
            host: vec![host("claude-opus-5-5", 2.0, time(2, 2, 2))],
            ..Rows::default()
        };
        let updated = update_body(&body, &new).unwrap();
        let range = section_range(&updated).unwrap().unwrap();
        assert_eq!(
            read(&updated[range]).unwrap(),
            Rows {
                tasks: vec![first()],
                host: new.host,
                ..Rows::default()
            }
        );
    }

    #[test]
    fn an_old_format_section_is_read_and_upgraded_keeping_rows_of_unknown_tasks() {
        let old = format!("Body\n\n{}\n", format_1(&[first(), second()]));
        let range = section_range(&old).unwrap().unwrap();
        assert_eq!(read(&old[range]).unwrap(), tasks(&[first(), second()]));

        // The record knows the second task; the first is older than the record.
        let mut known = second();
        known.usage.runs = 5;
        known.time = time(MINUTE, 0, 0);
        let ours = Rows {
            tasks: vec![known.clone()],
            host: vec![host("claude-opus-5-5", 1.0, time(MINUTE, 0, 0))],
            ..Rows::default()
        };
        let upgraded = update_body(&old, &ours).unwrap();
        let range = section_range(&upgraded).unwrap().unwrap();
        assert_eq!(
            read(&upgraded[range.clone()]).unwrap(),
            Rows {
                tasks: vec![first(), known],
                host: ours.host.clone(),
                ..Rows::default()
            }
        );
        assert!(
            upgraded[range].contains(&format!("{DATA_START}\nversion = 2\n")),
            "{upgraded}"
        );
        assert!(upgraded.starts_with("Body\n\n"), "{upgraded}");
        assert!(
            table(&upgraded)
                .iter()
                .any(|line| line.starts_with("| 3f9a1c2e | dev01 |")
                    && line.ends_with("| n/a | n/a | n/a |")),
            "{upgraded}"
        );
    }

    #[test]
    fn unpriced_host_responses_mark_their_cost_and_the_total_as_incomplete() {
        let mut fast = host("claude-opus-5-5", 1.5, time(MINUTE, 0, 0));
        fast.unpriced = Some(3);
        let mut timed = first();
        timed.time = time(MINUTE, 0, 0);
        let rows = Rows {
            tasks: vec![timed],
            host: vec![fast],
            ..Rows::default()
        };
        let body = update_body("", &rows).unwrap();
        assert_eq!(
            table(&body)[3..],
            [
                "| host | host | claude-opus-5-5 | | $1.5000 + 3 unpriced | 89.9% | 100 | 2,000 | 90,000 | 10,000 | 1m 00s | 0s | 0s |",
                "| **Total** | | | **3** | **$5.6873 (incomplete)** | **94.1%** | **1,304** | **50,310** | **6,210,455** | **381,902** | **2m 00s** | **0s** | **0s** |",
            ]
        );
        // The count reads back with the rows.
        let range = section_range(&body).unwrap().unwrap();
        assert_eq!(read(&body[range]).unwrap().host[0].unpriced, Some(3));
    }

    #[test]
    fn a_carried_row_of_a_task_the_record_has_in_another_item_is_dropped() {
        let theirs = update_body("", &tasks(&[first(), second()])).unwrap();
        // The record knows the second task, now in another item; the first it does not know.
        let ours = Rows {
            known: [second().task].into_iter().collect(),
            ..Rows::default()
        };
        let body = update_body(&theirs, &ours).unwrap();
        let range = section_range(&body).unwrap().unwrap();
        assert_eq!(read(&body[range]).unwrap().tasks, [first()]);
    }

    #[test]
    fn data_of_a_newer_format_is_refused() {
        let body = format!("{START}\n{DATA_START}\nversion = 3\n-->\n{END}");
        let error = format!("{:#}", update_body(&body, &tasks(&[first()])).unwrap_err());
        assert!(error.contains("format 3"), "{error}");
    }

    /// The data comment as a guest of format 1 read it: no version, no host rows, unknown
    /// fields refused.
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct GuestData {
        #[serde(default)]
        task: Vec<GuestRow>,
    }

    #[derive(Debug, Deserialize)]
    struct GuestRow {
        task: String,
        agent: String,
        models: Vec<String>,
        usage: TaskUsage,
    }

    #[test]
    fn an_old_guest_refuses_a_host_written_section_and_reads_its_own() {
        let ours = Rows {
            tasks: vec![first()],
            host: Vec::new(),
            ..Rows::default()
        };
        for rows in [
            ours.clone(),
            Rows {
                host: vec![host("claude-opus-5-5", 1.0, time(1, 1, 1))],
                ..ours
            },
        ] {
            let body = update_body("", &rows).unwrap();
            let at = body.find(DATA_START).unwrap() + DATA_START.len();
            let data = &body[at..at + body[at..].find(COMMENT_END).unwrap()];
            let error = toml::from_str::<GuestData>(data).unwrap_err().to_string();
            assert!(error.contains("unknown field `version`"), "{error}");
        }
        // What guests wrote, it reads.
        let old = format_1(&[first()]);
        let at = old.find(DATA_START).unwrap() + DATA_START.len();
        let data = &old[at..at + old[at..].find(COMMENT_END).unwrap()];
        let read: GuestData = toml::from_str(data).unwrap();
        let guest = &read.task[0];
        assert_eq!(
            (
                guest.task.as_str(),
                guest.agent.as_str(),
                guest.models.len(),
                guest.usage.runs
            ),
            ("3f9a1c2e", "dev01", 2, 3)
        );
    }

    #[test]
    fn markers_out_of_order_or_twice_are_refused() {
        let section = update_body("", &tasks(&[first()])).unwrap();
        for body in [
            format!("{START}\nno end"),
            format!("no start\n{END}"),
            format!("{END}\n{START}"),
            format!("{section}\n{section}"),
        ] {
            let error = update_body(&body, &tasks(&[first()]))
                .unwrap_err()
                .to_string();
            assert!(error.contains("markers"), "{error}");
        }
    }

    #[test]
    fn markers_mentioned_inline_are_left_alone_and_the_section_is_appended() {
        let body = format!(
            "The host rewrites everything between `{START}` and `{END}` in the open pull \
             requests.\n"
        );
        let updated = update_body(&body, &tasks(&[first()])).unwrap();
        assert!(
            updated.starts_with(&format!("{}\n\n{START}\n", body.trim_end())),
            "{updated}"
        );
        assert!(updated.ends_with(&format!("{END}\n")), "{updated}");
        assert_eq!(update_body(&updated, &tasks(&[first()])).unwrap(), updated);
    }

    #[test]
    fn markers_in_a_fenced_code_block_are_ignored() {
        for fence in ["```", "~~~", "````toml"] {
            let close = &fence[..fence
                .find(|c: char| c.is_alphabetic())
                .unwrap_or(fence.len())];
            let body = format!("Example:\n\n{fence}\n{START}\nrows\n{END}\n{close}\n");
            let updated = update_body(&body, &tasks(&[first()])).unwrap();
            assert!(
                updated.starts_with(&format!("{}\n\n{START}\n", body.trim_end())),
                "{updated}"
            );
            assert_eq!(updated.matches(START).count(), 2, "{updated}");
        }
    }

    #[test]
    fn a_section_after_an_inline_mention_is_found_and_replaced() {
        let mention = format!("Kept between `{START}` and `{END}`.\r\n\r\n");
        let body = format!("{mention}  {START}\r\n## Generation\r\nby hand\r\n{END} \r\nAfter\r\n");
        let updated = update_body(&body, &tasks(&[first()])).unwrap();
        assert!(!updated.contains("by hand"), "{updated}");
        assert!(
            updated.starts_with(&format!("{mention}  {START}\n")),
            "{updated}"
        );
        assert!(
            updated.ends_with(&format!("{END} \r\nAfter\r\n")),
            "{updated}"
        );
        assert!(
            table(&updated)
                .iter()
                .any(|line| line.starts_with("| 3f9a1c2e |"))
        );
    }

    #[test]
    fn a_section_without_data_is_rewritten_with_ours() {
        let body = format!("Text\n{START}\n## Generation\nfilled in by hand\n{END}\nAfter\n");
        let updated = update_body(&body, &tasks(&[first()])).unwrap();
        assert!(!updated.contains("filled in by hand"), "{updated}");
        assert!(updated.starts_with(&format!("Text\n{START}")), "{updated}");
        assert!(updated.ends_with(&format!("{END}\nAfter\n")), "{updated}");
    }

    #[test]
    fn data_that_does_not_parse_is_refused() {
        let body = format!("{START}\n{DATA_START}\n[[task]]\nnot toml\n-->\n{END}");
        assert!(update_body(&body, &tasks(&[first()])).is_err());
        let open = format!("{START}\n{DATA_START}\n{END}");
        assert!(update_body(&open, &tasks(&[first()])).is_err());
    }

    #[test]
    fn rows_that_could_break_the_table_or_the_comment_are_refused() {
        for (task, agent, model) in [
            ("a|b", "dev01", "claude-opus-5-5"),
            ("t1", "Dev 01", "claude-opus-5-5"),
            ("t1", "dev01", "evil-->"),
            ("t1", "dev01", "a|b"),
            ("", "dev01", "claude-opus-5-5"),
        ] {
            let bad = row(task, agent, &[model], TaskUsage::default());
            assert!(
                update_body("", &tasks(&[bad])).is_err(),
                "{task} {agent} {model}"
            );
        }
        let bad_host = Rows {
            tasks: Vec::new(),
            host: vec![host("evil-->", 0.0, Time::default())],
            ..Rows::default()
        };
        assert!(update_body("", &bad_host).is_err());
        // The same goes for rows read back from a body.
        let mut body = update_body("", &tasks(&[first()])).unwrap();
        body = body.replace("dev01", "Dev 01");
        assert!(update_body(&body, &tasks(&[second()])).is_err());
    }

    #[test]
    fn cache_hit_is_na_without_input() {
        let body =
            update_body("", &tasks(&[row("t1", "dev01", &[], TaskUsage::default())])).unwrap();
        assert!(
            table(&body).contains(
                &"| t1 | dev01 |  | 0 | $0.0000 | n/a | 0 | 0 | 0 | 0 | n/a | n/a | n/a |"
            ),
            "{body}"
        );
    }

    #[test]
    fn a_generation_heading_or_table_outside_the_markers_is_hand_written() {
        let section = update_body("", &tasks(&[first()])).unwrap();
        // The section's own heading and rows do not count, nor a quoted example.
        assert!(!hand_written(&format!("Body\n\n{section}")).unwrap());
        assert!(
            !hand_written("Body\n\n```\n## Generation\n| host | not measured |\n```\n").unwrap()
        );
        assert!(!hand_written("The Generation section is kept by the host.\n").unwrap());
        for outside in [
            "## Generation\n\nhost work by hand\n",
            "### generation\n",
            "| host (Opus 5.5) | not measured |\n",
            "| Host: Not Measured | | |\n",
        ] {
            assert!(
                hand_written(&format!("Body\n\n{outside}\n{section}")).unwrap(),
                "{outside}"
            );
            assert!(hand_written(outside).unwrap(), "{outside}");
        }
    }

    #[test]
    fn numbers_are_grouped_in_threes() {
        for (number, shown) in [
            (0, "0"),
            (999, "999"),
            (1000, "1,000"),
            (123_456_789, "123,456,789"),
            (u64::MAX, "18,446,744,073,709,551,615"),
        ] {
            assert_eq!(grouped(number), shown);
        }
    }
}
