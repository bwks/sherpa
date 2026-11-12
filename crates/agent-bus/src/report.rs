//! The usage report of `agent-bus usage`: one row per task from the record, and a total row.

use shared::util::{TableLayout, render_table};

use crate::message::{TaskUsage, cache_hit_permille};
use crate::store::TaskState;

const HEADERS: [&str; 8] = [
    "Task",
    "Agent",
    "Model",
    "Runs",
    "Cost (not billed)",
    "Cache hit",
    "Tokens",
    "State",
];
/// The columns of numbers, which are right-aligned: runs, cost, cache hit and tokens.
const NUMBER_COLUMNS: [usize; 4] = [3, 4, 5, 6];

/// One task in the report.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub task: String,
    pub agent: String,
    /// The model the task ran on, from its outcome messages. `None` for a task from before the
    /// guest named it.
    pub model: Option<String>,
    /// The task's totals as of its last outcome message that carried them. `None` for a task
    /// from before usage was tracked.
    pub usage: Option<TaskUsage>,
    /// Where the task is, from its last outcome message.
    pub state: TaskState,
}

/// The report as a table, or a sentence when there is nothing to report.
pub fn table(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "No usage recorded".to_owned();
    }
    let mut total = TaskUsage::default();
    let mut cells: Vec<[String; 8]> = rows
        .iter()
        .map(|row| {
            if let Some(usage) = &row.usage {
                total.runs = total.runs.saturating_add(usage.runs);
                total.cost_usd += usage.cost_usd;
                total.input = total.input.saturating_add(usage.input);
                total.output = total.output.saturating_add(usage.output);
                total.cache_creation = total.cache_creation.saturating_add(usage.cache_creation);
                total.cache_read = total.cache_read.saturating_add(usage.cache_read);
            }
            line(
                &row.task,
                &row.agent,
                row.model.as_deref().unwrap_or("n/a"),
                row.usage.as_ref(),
                row.state.as_str(),
            )
        })
        .collect();
    cells.push(line("total", "", "", Some(&total), ""));
    render_table(
        HEADERS,
        &cells,
        &TableLayout {
            right: &NUMBER_COLUMNS,
            ..TableLayout::default()
        },
    )
}

fn line(
    task: &str,
    agent: &str,
    model: &str,
    usage: Option<&TaskUsage>,
    state: &str,
) -> [String; 8] {
    let na = || "n/a".to_owned();
    let (runs, cost, hit, tokens) = match usage {
        Some(usage) => (
            usage.runs.to_string(),
            format!("${:.4}", usage.cost_usd),
            match cache_hit_permille(usage.input, usage.cache_creation, usage.cache_read) {
                Some(permille) => format!("{}.{}%", permille / 10, permille % 10),
                None => na(),
            },
            usage.tokens().to_string(),
        ),
        None => (na(), na(), na(), na()),
    };
    [
        task.to_owned(),
        agent.to_owned(),
        model.to_owned(),
        runs,
        cost,
        hit,
        tokens,
        state.to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(runs: u64, cost_usd: f64, input: u64, cache_read: u64) -> TaskUsage {
        TaskUsage {
            runs,
            cost_usd,
            input,
            output: 5,
            cache_read,
            ..TaskUsage::default()
        }
    }

    fn row(task: &str, model: Option<&str>, usage: Option<TaskUsage>, state: TaskState) -> Row {
        Row {
            task: task.to_owned(),
            agent: "dev".to_owned(),
            model: model.map(str::to_owned),
            usage,
            state,
        }
    }

    #[test]
    fn the_model_column_shows_the_model_each_task_ran_on() {
        let rows = [
            row(
                "a",
                Some("claude-sonnet-5-5"),
                Some(used(1, 0.04, 1, 1)),
                TaskState::Completed,
            ),
            row(
                "b",
                Some("claude-haiku-5-5"),
                Some(used(1, 0.04, 1, 1)),
                TaskState::Completed,
            ),
            row("old", None, Some(used(1, 0.04, 1, 1)), TaskState::Completed),
        ];
        let table = table(&rows);
        assert!(table.contains("Model"), "{table}");
        for (task, model) in [
            ("a", "claude-sonnet-5-5"),
            ("b", "claude-haiku-5-5"),
            ("old", "n/a"),
        ] {
            let line = table
                .lines()
                .find(|line| line.contains(&format!(" {task} ")))
                .unwrap_or_default();
            assert!(line.contains(model), "{line}");
        }
    }

    #[test]
    fn the_table_has_a_row_per_task_and_a_total() {
        let rows = [
            row("a", None, Some(used(2, 0.25, 10, 90)), TaskState::Completed),
            row("b", None, Some(used(1, 0.5, 0, 0)), TaskState::Canceled),
        ];
        let table = table(&rows);
        for expected in [
            "Task",
            "Cache hit",
            "$0.2500",
            "90.0%",
            "completed",
            "canceled",
        ] {
            assert!(table.contains(expected), "{expected} missing from\n{table}");
        }
        let total = table
            .lines()
            .find(|line| line.contains("total"))
            .unwrap_or_default();
        // 3 runs, $0.7500, 90 of 100 input tokens from the cache, 110 tokens (10+90+5+5)
        assert!(total.contains('3'), "{total}");
        assert!(total.contains("$0.7500"), "{total}");
        assert!(total.contains("90.0%"), "{total}");
        assert!(total.contains("110"), "{total}");
    }

    #[test]
    fn the_cells_of_a_task_without_usage_show_n_a() {
        let cells = line("t", "dev", "n/a", None, "failed");
        assert_eq!(
            cells,
            ["t", "dev", "n/a", "n/a", "n/a", "n/a", "n/a", "failed"]
        );
        assert_eq!(table(&[]), "No usage recorded");
    }

    #[test]
    fn a_task_without_usage_is_not_totalled() {
        let rows = [
            row("old", None, None, TaskState::Failed),
            row(
                "new",
                None,
                Some(used(2, 0.25, 10, 90)),
                TaskState::Completed,
            ),
        ];
        let table = table(&rows);
        let old = table
            .lines()
            .find(|l| l.contains("old"))
            .unwrap_or_default();
        assert!(old.contains("n/a"), "{old}");
        assert!(!old.contains("$0.0000"), "{old}");
        let total = table
            .lines()
            .find(|l| l.contains("total"))
            .unwrap_or_default();
        assert!(total.contains("$0.2500"), "{total}");
        assert!(total.contains('2'), "{total}");
    }
}
