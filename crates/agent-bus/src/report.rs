//! The usage report of `agent-bus usage`: one row per task from the outcome messages the host
//! received, and a total row.

use shared::util::render_modern_table;

use crate::message::{Kind, Message, TaskUsage, cache_hit_permille};

const HEADERS: [&str; 7] = [
    "Task",
    "Agent",
    "Runs",
    "Cost (not billed)",
    "Cache hit",
    "Tokens",
    "State",
];
/// The columns of numbers, which are right-aligned: runs, cost, cache hit and tokens.
const NUMBER_COLUMNS: [usize; 4] = [2, 3, 4, 5];

/// One task in the report.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub task: String,
    pub agent: String,
    /// The task's totals as of its last outcome message that carried them. `None` for a task
    /// from before usage was tracked.
    pub usage: Option<TaskUsage>,
    /// The kind of the task's last outcome message.
    pub state: Kind,
}

/// The messages of `task`, oldest first, of every kind.
pub fn task_messages<'a>(messages: &'a [Message], task: &str) -> Vec<&'a Message> {
    messages
        .iter()
        .filter(|message| message.task == task)
        .collect()
}

/// The tasks in `messages`, in the order they first had an outcome, for `agent` only when given.
/// A task's usage is cumulative, so its latest figures are the ones that count. Messages that are
/// not outcomes (tasks, answers, progress) are skipped. Tasks without usage are left out unless
/// `all`.
pub fn rows(messages: &[Message], agent: Option<&str>, all: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::new();
    for message in messages {
        let outcome = matches!(
            message.kind,
            Kind::Completed | Kind::Question | Kind::Failed | Kind::Canceled
        );
        if !outcome || agent.is_some_and(|wanted| wanted != message.from) {
            continue;
        }
        let position = rows.iter().position(|row| row.task == message.task);
        let index = position.unwrap_or_else(|| {
            rows.push(Row {
                task: message.task.clone(),
                agent: message.from.clone(),
                usage: None,
                state: message.kind,
            });
            rows.len() - 1
        });
        let Some(row) = rows.get_mut(index) else {
            continue;
        };
        row.state = message.kind;
        if let Some(usage) = &message.task_usage {
            row.usage = Some(usage.clone());
        }
    }
    if !all {
        rows.retain(|row| row.usage.is_some());
    }
    rows
}

/// The report as a table, or a sentence when there is nothing to report.
pub fn table(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "No usage recorded".to_owned();
    }
    let mut total = TaskUsage::default();
    let mut cells: Vec<Vec<String>> = rows
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
                row.usage.as_ref(),
                &format!("{:?}", row.state).to_lowercase(),
            )
        })
        .collect();
    cells.push(line("total", "", Some(&total), ""));
    render_modern_table(&HEADERS, &cells, &NUMBER_COLUMNS)
}

fn line(task: &str, agent: &str, usage: Option<&TaskUsage>, state: &str) -> Vec<String> {
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
    vec![
        task.to_owned(),
        agent.to_owned(),
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

    fn outcome(task: &str, from: &str, kind: Kind, usage: Option<TaskUsage>) -> Message {
        Message {
            task: task.to_owned(),
            from: from.to_owned(),
            to: "host".to_owned(),
            kind,
            text: String::new(),
            usage: None,
            task_usage: usage,
            github: false,
        }
    }

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

    #[test]
    fn a_task_has_one_row_with_its_latest_totals_and_state() {
        let messages = [
            outcome("a", "dev", Kind::Progress, None),
            outcome("a", "dev", Kind::Question, Some(used(1, 0.04, 10, 30))),
            outcome("b", "test", Kind::Failed, None),
            outcome("a", "dev", Kind::Completed, Some(used(2, 0.06, 20, 60))),
        ];
        let rows = rows(&messages, None, true);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].task, "a");
        assert_eq!(rows[0].state, Kind::Completed);
        assert_eq!(rows[0].usage, Some(used(2, 0.06, 20, 60)));
        assert_eq!(rows[1].task, "b");
        assert_eq!(rows[1].state, Kind::Failed);
        assert_eq!(rows[1].usage, None);
    }

    #[test]
    fn the_agent_filter_keeps_that_agents_tasks() {
        let messages = [
            outcome("a", "dev", Kind::Completed, Some(used(1, 0.04, 1, 1))),
            outcome("b", "test", Kind::Completed, Some(used(1, 0.04, 1, 1))),
        ];
        let rows = rows(&messages, Some("test"), true);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task, "b");
        assert_eq!(super::rows(&messages, Some("nobody"), true).len(), 0);
    }

    #[test]
    fn the_table_has_a_row_per_task_and_a_total() {
        let messages = [
            outcome("a", "dev", Kind::Completed, Some(used(2, 0.25, 10, 90))),
            outcome("b", "dev", Kind::Canceled, Some(used(1, 0.5, 0, 0))),
        ];
        let table = table(&rows(&messages, None, false));
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
        let cells = line("t", "dev", None, "failed");
        assert_eq!(cells, ["t", "dev", "n/a", "n/a", "n/a", "n/a", "failed"]);
        assert_eq!(table(&[]), "No usage recorded");
    }

    #[test]
    fn tasks_without_usage_are_listed_only_with_all_and_not_totalled() {
        let messages = [
            outcome("old", "dev", Kind::Failed, None),
            outcome("new", "dev", Kind::Completed, Some(used(2, 0.25, 10, 90))),
        ];
        let default = rows(&messages, None, false);
        assert_eq!(default.len(), 1);
        assert_eq!(default[0].task, "new");

        let all = rows(&messages, None, true);
        assert_eq!(all.len(), 2);
        let table = table(&all);
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

    #[test]
    fn a_tasks_messages_are_all_kinds_in_order() {
        let messages = [
            outcome("a", "dev", Kind::Progress, None),
            outcome("b", "dev", Kind::Completed, None),
            outcome("a", "dev", Kind::Question, None),
        ];
        let kinds: Vec<Kind> = task_messages(&messages, "a")
            .iter()
            .map(|m| m.kind)
            .collect();
        assert_eq!(kinds, [Kind::Progress, Kind::Question]);
        assert_eq!(task_messages(&messages, "zzz").len(), 0);
    }
}
