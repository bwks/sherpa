//! The `## Generation` section of a pull request body: what the agent tasks behind the pull
//! request cost, kept by the guest agent between two markers. Pure text and data; the ledger
//! (`ledger`) supplies the rows and `pull_request` reads and writes the bodies.
//!
//! The section carries its rows twice: as a markdown table for people, and as TOML in an HTML
//! comment, which is what is read back. Guests in other VMs keep their own ledgers, so a guest
//! rewriting the section keeps the rows of tasks it does not know and replaces the rows of its
//! own tasks, whose figures are cumulative. The table is never parsed.

use std::ops::Range;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::message::{TaskUsage, cache_hit_permille, check_agent_name};

/// The line that opens the section.
pub(super) const START: &str = "<!-- agent-bus:generation -->";
/// The line that closes the section.
pub(super) const END: &str = "<!-- /agent-bus:generation -->";
/// Opens the HTML comment holding the rows as TOML.
const DATA_START: &str = "<!-- agent-bus:generation-data";
/// Closes an HTML comment.
const COMMENT_END: &str = "-->";
const HEADING: &str = "## Generation";
const NOTE: &str = "Figures from agent-bus: list-price cost (Claude Code's `costUSD`), not \
billed: guest agents run on the Claude subscription. Work done outside the agent bus is not \
measured.";
const TABLE_HEAD: &str = "| Task | Agent | Models | Runs | Cost (USD) | Cache hit | Input | \
Output | Cache read | Cache write |\n|---|---|---|--:|--:|--:|--:|--:|--:|--:|";
/// The longest task ID, model name or model list kept.
const MAX_ID: usize = 64;
const MAX_MODELS: usize = 16;

/// An open fenced code block, as in `CommonMark`: three or more backticks or tildes after at
/// most three spaces. Holds its character and how many times it repeats.
struct Fence {
    mark: char,
    len: usize,
}

/// One task in the section.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(super) struct Row {
    pub(super) task: String,
    pub(super) agent: String,
    /// Every model its runs used, subagents' included, sorted.
    pub(super) models: Vec<String>,
    /// Its totals over all its runs.
    pub(super) usage: TaskUsage,
}

/// The TOML in the data comment.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    #[serde(default)]
    task: Vec<Row>,
}

/// `body` with its section rewritten to hold `ours` and the rows of other tasks it already had,
/// or with the section appended when it has none. Everything outside the markers is unchanged.
///
/// # Errors
///
/// Fails when the markers are not one start followed by one end, the data in the section cannot
/// be read, or a row has a task, agent or model that cannot be shown safely.
pub(super) fn update_body(body: &str, ours: &[Row]) -> Result<String> {
    for row in ours {
        check_row(row)?;
    }
    let Some(range) = section_range(body)? else {
        let section = render(ours)?;
        let kept = body.trim_end();
        return if kept.is_empty() {
            Ok(format!("{section}\n"))
        } else {
            Ok(format!("{kept}\n\n{section}\n"))
        };
    };
    let theirs = rows_in(&body[range.clone()])?;
    let section = render(&merge(theirs, ours))?;
    Ok(format!(
        "{}{section}{}",
        &body[..range.start],
        &body[range.end..]
    ))
}

/// Where the section is in `body`, from the start of its first marker to the end of its last;
/// `None` when it has no markers. A marker counts only when it is a line of its own, apart from
/// surrounding whitespace, and is not inside a fenced code block, so a description that quotes
/// the markers is not taken for the section.
fn section_range(body: &str) -> Result<Option<Range<usize>>> {
    let mut starts = Vec::new();
    let mut ends = Vec::new();
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

/// The rows in the data comment of `section`; none when it has no data comment.
fn rows_in(section: &str) -> Result<Vec<Row>> {
    let Some(at) = section.find(DATA_START) else {
        return Ok(Vec::new());
    };
    let data = &section[at + DATA_START.len()..];
    let end = data
        .find(COMMENT_END)
        .context("The Generation data comment in the pull request body is not closed")?;
    let data: Data = toml::from_str(&data[..end])
        .context("Unable to read the Generation data in the pull request body")?;
    for row in &data.task {
        check_row(row)?;
    }
    Ok(data.task)
}

/// `theirs` in their order, each replaced by the row of `ours` for the same task, then the rest
/// of `ours` in its order.
fn merge(theirs: Vec<Row>, ours: &[Row]) -> Vec<Row> {
    let mut merged: Vec<Row> = theirs
        .into_iter()
        .map(|row| {
            ours.iter()
                .find(|our| our.task == row.task)
                .cloned()
                .unwrap_or(row)
        })
        .collect();
    for row in ours {
        if !merged.iter().any(|kept| kept.task == row.task) {
            merged.push(row.clone());
        }
    }
    merged
}

/// The section, from its start marker to its end marker.
fn render(rows: &[Row]) -> Result<String> {
    let mut total = TaskUsage::default();
    let mut lines = vec![
        START.to_owned(),
        HEADING.to_owned(),
        String::new(),
        NOTE.to_owned(),
        String::new(),
        TABLE_HEAD.to_owned(),
    ];
    for row in rows {
        let usage = &row.usage;
        total.runs = total.runs.saturating_add(usage.runs);
        total.cost_usd += usage.cost_usd;
        total.input = total.input.saturating_add(usage.input);
        total.output = total.output.saturating_add(usage.output);
        total.cache_creation = total.cache_creation.saturating_add(usage.cache_creation);
        total.cache_read = total.cache_read.saturating_add(usage.cache_read);
        let cells = figures(usage);
        lines.push(format!(
            "| {} | {} | {} | {} |",
            row.task,
            row.agent,
            row.models.join(", "),
            cells.join(" | ")
        ));
    }
    let totals: Vec<String> = figures(&total)
        .into_iter()
        .map(|cell| format!("**{cell}**"))
        .collect();
    lines.push(format!("| **Total** | | | {} |", totals.join(" | ")));
    lines.push(String::new());
    // Every string in the rows is checked to hold no `-->` (`check_row`), so the data cannot
    // close the comment early.
    let data = toml::to_string(&Data {
        task: rows.to_vec(),
    })
    .context("Unable to write the Generation data")?;
    lines.push(format!("{DATA_START}\n{data}{COMMENT_END}"));
    lines.push(END.to_owned());
    Ok(lines.join("\n"))
}

/// The number cells of a row: runs, cost, cache hit, input, output, cache read, cache write.
fn figures(usage: &TaskUsage) -> Vec<String> {
    let hit = match cache_hit_permille(usage.input, usage.cache_creation, usage.cache_read) {
        Some(permille) => format!("{}.{}%", permille / 10, permille % 10),
        None => "n/a".to_owned(),
    };
    vec![
        grouped(usage.runs),
        format!("${:.4}", usage.cost_usd),
        hit,
        grouped(usage.input),
        grouped(usage.output),
        grouped(usage.cache_read),
        grouped(usage.cache_creation),
    ]
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

/// Refuse a row whose task, agent or models could break the table or the comment: each must be
/// short and of letters, digits and `-._`, models also `:/@[]`.
fn check_row(row: &Row) -> Result<()> {
    let id = |text: &str, extra: &[char]| {
        !text.is_empty()
            && text.len() <= MAX_ID
            && text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c) || extra.contains(&c))
    };
    ensure!(
        id(&row.task, &[]),
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
        ensure!(
            id(model, &[':', '/', '@', '[', ']']),
            "The model '{}' of task {} cannot go in a Generation section",
            model.escape_debug(),
            row.task
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(task: &str, agent: &str, models: &[&str], usage: TaskUsage) -> Row {
        Row {
            task: task.to_owned(),
            agent: agent.to_owned(),
            models: models.iter().map(|model| (*model).to_owned()).collect(),
            usage,
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

    /// The table part of a section: its lines that start with `|`.
    fn table(body: &str) -> Vec<&str> {
        body.lines().filter(|line| line.starts_with('|')).collect()
    }

    #[test]
    fn two_tasks_render_as_a_table_with_a_total() {
        let body = update_body("", &[first(), second()]).unwrap();
        assert!(
            body.starts_with(&format!("{START}\n## Generation\n\n")),
            "{body}"
        );
        assert!(body.contains("list-price cost"), "{body}");
        assert!(body.contains("not billed"), "{body}");
        assert_eq!(
            table(&body),
            [
                "| Task | Agent | Models | Runs | Cost (USD) | Cache hit | Input | Output | Cache read | Cache write |",
                "|---|---|---|--:|--:|--:|--:|--:|--:|--:|",
                "| 3f9a1c2e | dev01 | claude-haiku-5-5, claude-opus-5-5 | 3 | $4.1873 | 94.2% | 1,204 | 48,310 | 6,120,455 | 371,902 |",
                "| 7b20d4aa | dev02 | claude-sonnet-5-5 | 1 | $0.6120 | 91.7% | 88 | 9,412 | 1,402,330 | 126,004 |",
                "| **Total** | | | **4** | **$4.7993** | **93.7%** | **1,292** | **57,722** | **7,522,785** | **497,906** |",
            ]
        );
        assert!(body.trim_end().ends_with(END), "{body}");
    }

    #[test]
    fn the_data_comment_reads_back_as_the_rows() {
        let body = update_body("", &[first(), second()]).unwrap();
        let range = section_range(&body).unwrap().unwrap();
        assert_eq!(rows_in(&body[range]).unwrap(), [first(), second()]);
    }

    #[test]
    fn a_body_without_the_section_gets_it_appended_after_a_blank_line() {
        let body =
            update_body("Fixes the thing.\n\n## Tests\n\nAll pass.\n\n", &[first()]).unwrap();
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
        let old = update_body("", &[first()]).unwrap();
        let body = format!("{before}{}{after}", old.trim_end());
        let mut changed = first();
        changed.usage.runs = 4;
        let updated = update_body(&body, &[changed]).unwrap();
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
        let once = update_body("Body\n", &[first(), second()]).unwrap();
        assert_eq!(update_body(&once, &[first(), second()]).unwrap(), once);
        assert_eq!(update_body(&once, &[second()]).unwrap(), once);
    }

    #[test]
    fn rows_of_other_tasks_are_kept_and_ours_replace_theirs() {
        let theirs = update_body("", &[first(), second()]).unwrap();
        let mut ours = second();
        ours.usage.runs = 2;
        let third = row(
            "c0ffee00",
            "dev01",
            &["claude-opus-5-5"],
            TaskUsage::default(),
        );
        let body = update_body(&theirs, &[third.clone(), ours.clone()]).unwrap();
        let range = section_range(&body).unwrap().unwrap();
        assert_eq!(rows_in(&body[range]).unwrap(), [first(), ours, third]);
    }

    #[test]
    fn markers_out_of_order_or_twice_are_refused() {
        let section = update_body("", &[first()]).unwrap();
        for body in [
            format!("{START}\nno end"),
            format!("no start\n{END}"),
            format!("{END}\n{START}"),
            format!("{section}\n{section}"),
        ] {
            let error = update_body(&body, &[first()]).unwrap_err().to_string();
            assert!(error.contains("markers"), "{error}");
        }
    }

    #[test]
    fn markers_mentioned_inline_are_left_alone_and_the_section_is_appended() {
        let body = format!(
            "The guest rewrites everything between `{START}` and `{END}` in the open pull \
             requests.\n"
        );
        let updated = update_body(&body, &[first()]).unwrap();
        assert!(
            updated.starts_with(&format!("{}\n\n{START}\n", body.trim_end())),
            "{updated}"
        );
        assert!(updated.ends_with(&format!("{END}\n")), "{updated}");
        assert_eq!(update_body(&updated, &[first()]).unwrap(), updated);
    }

    #[test]
    fn markers_in_a_fenced_code_block_are_ignored() {
        for fence in ["```", "~~~", "````toml"] {
            let close = &fence[..fence
                .find(|c: char| c.is_alphabetic())
                .unwrap_or(fence.len())];
            let body = format!("Example:\n\n{fence}\n{START}\nrows\n{END}\n{close}\n");
            let updated = update_body(&body, &[first()]).unwrap();
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
        let updated = update_body(&body, &[first()]).unwrap();
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
        let updated = update_body(&body, &[first()]).unwrap();
        assert!(!updated.contains("filled in by hand"), "{updated}");
        assert!(updated.starts_with(&format!("Text\n{START}")), "{updated}");
        assert!(updated.ends_with(&format!("{END}\nAfter\n")), "{updated}");
    }

    #[test]
    fn data_that_does_not_parse_is_refused() {
        let body = format!("{START}\n{DATA_START}\n[[task]]\nnot toml\n-->\n{END}");
        assert!(update_body(&body, &[first()]).is_err());
        let open = format!("{START}\n{DATA_START}\n{END}");
        assert!(update_body(&open, &[first()]).is_err());
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
            assert!(update_body("", &[bad]).is_err(), "{task} {agent} {model}");
        }
        // The same goes for rows read back from a body.
        let mut body = update_body("", &[first()]).unwrap();
        body = body.replace("dev01", "Dev 01");
        assert!(update_body(&body, &[second()]).is_err());
    }

    #[test]
    fn cache_hit_is_na_without_input() {
        let body = update_body("", &[row("t1", "dev01", &[], TaskUsage::default())]).unwrap();
        assert!(
            table(&body).contains(&"| t1 | dev01 |  | 0 | $0.0000 | n/a | 0 | 0 | 0 | 0 |"),
            "{body}"
        );
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
