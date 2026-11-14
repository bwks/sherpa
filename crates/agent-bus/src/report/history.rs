//! The table of `agent-bus history`: one row per task from the record.

use jiff::Timestamp;
use shared::util::{TableLayout, render_table};

use crate::store::HistoryRow;

const HEADERS: [&str; 7] = [
    "Task",
    "Agent",
    "State",
    "Model",
    "Last active (UTC)",
    "Cost (not billed)",
    "Prompt",
];
/// The cost column is right-aligned.
const NUMBER_COLUMNS: [usize; 1] = [5];
/// How much of a task's prompt the history shows: the start of its first line.
const PROMPT_CHARS: usize = 60;

/// The history table, or a sentence when there are no tasks.
pub fn table(rows: &[HistoryRow]) -> String {
    if rows.is_empty() {
        return "No tasks recorded".to_owned();
    }
    let cells: Vec<[String; 7]> = rows.iter().map(row_cells).collect();
    render_table(
        HEADERS,
        &cells,
        &TableLayout {
            right: &NUMBER_COLUMNS,
            ..TableLayout::default()
        },
    )
}

fn row_cells(row: &HistoryRow) -> [String; 7] {
    let none = || "-".to_owned();
    [
        row.task.clone(),
        row.agent.clone(),
        row.state.as_str().to_owned(),
        row.model.clone().unwrap_or_else(|| "default".to_owned()),
        utc_minute(row.last_at_ms),
        row.cost_usd.map_or_else(none, |cost| format!("${cost:.4}")),
        row.prompt.as_deref().map_or_else(none, prompt_start),
    ]
}

/// `at_ms`, milliseconds since the Unix epoch, as `YYYY-MM-DD HH:MM` in UTC.
pub(super) fn utc_minute(at_ms: u64) -> String {
    i64::try_from(at_ms)
        .ok()
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .map_or_else(
            || "?".to_owned(),
            |at| at.strftime("%Y-%m-%d %H:%M").to_string(),
        )
}

/// The start of `prompt`'s first line, with `…` when there is more.
pub(super) fn prompt_start(prompt: &str) -> String {
    let first = prompt.lines().next().unwrap_or_default();
    let start: String = first.chars().take(PROMPT_CHARS).collect();
    if start.len() < prompt.trim_end().len() {
        format!("{start}…")
    } else {
        start
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TaskState;

    fn task(task: &str, prompt: Option<&str>) -> HistoryRow {
        HistoryRow {
            task: task.to_owned(),
            agent: "dev".to_owned(),
            state: TaskState::Completed,
            model: Some("claude-opus-5-5".to_owned()),
            github: false,
            prompt: prompt.map(str::to_owned),
            sent_at_ms: Some(1_794_300_000_000),
            last_at_ms: 1_794_300_060_000,
            runs: Some(2),
            cost_usd: Some(0.5),
        }
    }

    #[test]
    fn each_task_has_a_row_with_the_start_of_its_prompt() {
        let long = format!("{}\nsecond line", "x".repeat(70));
        let rows = [
            task("a", Some("Fix the bug\nDetails follow")),
            task("b", Some(&long)),
            HistoryRow {
                model: None,
                cost_usd: None,
                ..task("c", None)
            },
        ];
        let table = table(&rows);
        for header in HEADERS {
            assert!(table.contains(header), "{header} missing from\n{table}");
        }
        let line = |name: &str| {
            table
                .lines()
                .find(|line| line.contains(&format!(" {name} ")))
                .unwrap_or_default()
                .to_owned()
        };
        let a = line("a");
        for cell in [
            "completed",
            "claude-opus-5-5",
            "2026-11-10 08:41",
            "$0.5000",
            "Fix the bug…",
        ] {
            assert!(a.contains(cell), "{cell} missing from {a}");
        }
        assert!(!a.contains("Details"), "{a}");
        assert!(line("b").contains(&format!("{}…", "x".repeat(PROMPT_CHARS))));
        let c = line("c");
        assert!(c.contains("default") && c.contains(" - "), "{c}");
    }

    #[test]
    fn a_one_line_prompt_is_shown_whole() {
        assert_eq!(prompt_start("Fix it"), "Fix it");
        assert_eq!(prompt_start("Fix it\n"), "Fix it");
        assert_eq!(table(&[]), "No tasks recorded");
    }
}
