//! The table of `agent-bus usage`: one row per task from the record, one per work item for the
//! host's own usage, and a total row.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use shared::util::{TableLayout, render_table};

use crate::message::{TaskUsage, cache_hit_permille};
use crate::store::TaskState;
use crate::store::host::HostRow;
use crate::timing::{Times, duration};

/// The agent and task the host's own rows are shown as.
pub const HOST: &str = "host";
/// The item cell of the host's usage that no item has.
pub const UNATTRIBUTED: &str = "unattributed";
const HEADERS: [&str; 12] = [
    "Task",
    "Agent",
    "Item",
    "Model",
    "Runs",
    "Cost (not billed)",
    "Cache hit",
    "Tokens",
    "Work",
    "Testing",
    "Waiting on user",
    "State",
];
/// The columns of numbers, which are right-aligned: runs to waiting on user.
const NUMBER_COLUMNS: [usize; 7] = [4, 5, 6, 7, 8, 9, 10];
/// A day, in milliseconds: the host's usage is added up by day for `usage --by`.
const DAY_MS: u64 = 86_400_000;

/// One row of the report: a task, or the host's usage in one item.
#[derive(Debug, PartialEq)]
pub struct Row {
    pub task: String,
    pub agent: String,
    /// The task's item, or the host usage's (`None`: unattributed).
    pub work: Option<i64>,
    /// The name of `work`, filled in before the table is printed.
    pub item: Option<String>,
    /// The model the task ran on, from its outcome messages; the host's models. `None` for a
    /// task from before the guest named it.
    pub model: Option<String>,
    /// The task's totals as of its last outcome message that carried them. `None` for a task
    /// from before usage was tracked.
    pub usage: Option<TaskUsage>,
    /// Where the task is, from its last outcome message; `None` for the host.
    pub state: Option<TaskState>,
    /// When it was sent, else its latest message: the week `usage --by week` counts it in.
    pub at_ms: u64,
    /// The time buckets: work and testing from the runs that reported them (`None` when none
    /// did), waiting on the user from its questions.
    pub time: Time,
    /// The host's responses with no price, whose cost is not in `usage`.
    pub unpriced: u64,
}

/// A row's time buckets, in milliseconds, each `None` when not known.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Time {
    pub work: Option<u64>,
    pub testing: Option<u64>,
    pub waiting: Option<u64>,
}

impl From<Times> for Time {
    fn from(times: Times) -> Self {
        Self {
            work: Some(times.work),
            testing: Some(times.testing),
            waiting: Some(times.waiting),
        }
    }
}

impl TaskUsage {
    /// All tokens of every kind.
    pub fn tokens(&self) -> u64 {
        self.input
            .saturating_add(self.output)
            .saturating_add(self.cache_creation)
            .saturating_add(self.cache_read)
    }
}

/// The host's usage as one row per item, oldest item first, with `times` its time by item.
/// Items with time but no responses get a row too.
pub fn host_rows(rows: &[HostRow], times: &HashMap<Option<i64>, Times>) -> Vec<Row> {
    let mut by_item: BTreeMap<Option<i64>, (TaskUsage, BTreeSet<&str>, u64, u64)> = BTreeMap::new();
    for row in rows {
        let (usage, models, unpriced, first) = by_item.entry(row.work).or_insert((
            TaskUsage::default(),
            BTreeSet::new(),
            0,
            row.at_ms,
        ));
        add_host(usage, row);
        models.insert(&row.model);
        if row.cost_usd.is_none() {
            *unpriced += 1;
        }
        *first = (*first).min(row.at_ms);
    }
    for item in times.keys() {
        by_item
            .entry(*item)
            .or_insert((TaskUsage::default(), BTreeSet::new(), 0, 0));
    }
    by_item
        .into_iter()
        .map(|(work, (usage, models, unpriced, at_ms))| Row {
            task: HOST.to_owned(),
            agent: HOST.to_owned(),
            work,
            item: None,
            model: (!models.is_empty()).then(|| models.into_iter().collect::<Vec<_>>().join(", ")),
            usage: Some(usage),
            state: None,
            at_ms,
            time: times.get(&work).copied().unwrap_or_default().into(),
            unpriced,
        })
        .collect()
}

/// The host's usage as rows for `usage --by`: one per model and UTC day, so each lands in its
/// model's and its week's group.
pub fn host_group_rows(rows: &[HostRow]) -> Vec<Row> {
    let mut by_day: BTreeMap<(&str, u64), (TaskUsage, u64)> = BTreeMap::new();
    for row in rows {
        let (usage, _) = by_day
            .entry((&row.model, row.at_ms / DAY_MS))
            .or_insert((TaskUsage::default(), row.at_ms));
        add_host(usage, row);
    }
    by_day
        .into_iter()
        .map(|((model, _), (usage, at_ms))| Row {
            task: HOST.to_owned(),
            agent: HOST.to_owned(),
            work: None,
            item: None,
            model: Some(model.to_owned()),
            usage: Some(usage),
            state: None,
            at_ms,
            time: Time::default(),
            unpriced: 0,
        })
        .collect()
}

/// Add one host response to `usage`; its cost when it has one.
fn add_host(usage: &mut TaskUsage, row: &HostRow) {
    usage.cost_usd += row.cost_usd.unwrap_or_default();
    usage.input = usage.input.saturating_add(row.tokens.input);
    usage.output = usage.output.saturating_add(row.tokens.output);
    usage.cache_creation = usage
        .cache_creation
        .saturating_add(row.tokens.cache_write_5m)
        .saturating_add(row.tokens.cache_write_1h);
    usage.cache_read = usage.cache_read.saturating_add(row.tokens.cache_read);
}

/// The report as a table, or a sentence when there is nothing to report.
pub fn table(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "No usage recorded".to_owned();
    }
    let mut total = TaskUsage::default();
    let mut time = Time::default();
    let mut unpriced = 0;
    let mut cells: Vec<[String; 12]> = rows
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
            for (sum, part) in [
                (&mut time.work, row.time.work),
                (&mut time.testing, row.time.testing),
                (&mut time.waiting, row.time.waiting),
            ] {
                if let Some(part) = part {
                    *sum = Some(sum.unwrap_or_default().saturating_add(part));
                }
            }
            unpriced += row.unpriced;
            let item = match (&row.item, row.state) {
                (Some(item), _) => item.clone(),
                (None, None) => UNATTRIBUTED.to_owned(),
                (None, Some(_)) => "-".to_owned(),
            };
            line(
                [
                    &row.task,
                    &row.agent,
                    &item,
                    row.model.as_deref().unwrap_or("n/a"),
                ],
                row.usage.as_ref(),
                row.state.is_some(),
                row.unpriced,
                row.time,
                row.state.map_or("-", TaskState::as_str),
            )
        })
        .collect();
    cells.push(line(
        ["total", "", "", ""],
        Some(&total),
        true,
        unpriced,
        time,
        "",
    ));
    render_table(
        HEADERS,
        &cells,
        &TableLayout {
            right: &NUMBER_COLUMNS,
            ..TableLayout::default()
        },
    )
}

/// One row's cells: `names` (task, agent, item, model), its usage with its runs when
/// `counts_runs`, its unpriced responses, its time and its state.
fn line(
    names: [&str; 4],
    usage: Option<&TaskUsage>,
    counts_runs: bool,
    unpriced: u64,
    time: Time,
    state: &str,
) -> [String; 12] {
    let na = || "n/a".to_owned();
    let (runs, cost, hit, tokens) = match usage {
        Some(usage) => (
            if counts_runs {
                usage.runs.to_string()
            } else {
                "-".to_owned()
            },
            if unpriced > 0 {
                format!("${:.4} +{unpriced} unpriced", usage.cost_usd)
            } else {
                format!("${:.4}", usage.cost_usd)
            },
            match cache_hit_permille(usage.input, usage.cache_creation, usage.cache_read) {
                Some(permille) => format!("{}.{}%", permille / 10, permille % 10),
                None => na(),
            },
            usage.tokens().to_string(),
        ),
        None => (na(), na(), na(), na()),
    };
    let shown = |ms: Option<u64>| ms.map_or_else(na, duration);
    let [task, agent, item, model] = names;
    [
        task.to_owned(),
        agent.to_owned(),
        item.to_owned(),
        model.to_owned(),
        runs,
        cost,
        hit,
        tokens,
        shown(time.work),
        shown(time.testing),
        shown(time.waiting),
        state.to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::price::Tokens;

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
            work: None,
            item: None,
            model: model.map(str::to_owned),
            usage,
            state: Some(state),
            at_ms: 0,
            time: Time::default(),
            unpriced: 0,
        }
    }

    #[test]
    fn tokens_add_every_kind_and_saturate() {
        let mut usage = used(1, 0.0, 4, 7);
        usage.cache_creation = 3;
        assert_eq!(usage.tokens(), 4 + 5 + 3 + 7);
        usage.input = u64::MAX;
        assert_eq!(usage.tokens(), u64::MAX);
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
        let cells = line(
            ["t", "dev", "-", "n/a"],
            None,
            true,
            0,
            Time::default(),
            "failed",
        );
        assert_eq!(
            cells,
            [
                "t", "dev", "-", "n/a", "n/a", "n/a", "n/a", "n/a", "n/a", "n/a", "n/a", "failed"
            ]
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

    #[test]
    fn host_rows_come_one_per_item_with_their_time_and_unpriced_responses() {
        let response = |work, model: &str, cost_usd| HostRow {
            work,
            model: model.to_owned(),
            at_ms: 1_000,
            tokens: Tokens {
                input: 10,
                output: 5,
                cache_read: 80,
                cache_write_5m: 0,
                cache_write_1h: 10,
            },
            cost_usd,
        };
        let responses = [
            response(Some(1), "claude-opus-5-5", Some(0.5)),
            response(Some(1), "claude-haiku-5-5", Some(0.25)),
            response(None, "claude-opus-5-5", None),
        ];
        let times = HashMap::from([
            (
                Some(1),
                Times {
                    work: 4_020_000,
                    testing: 750_000,
                    waiting: 0,
                },
            ),
            (Some(2), Times::default()),
        ]);
        let mut rows = host_rows(&responses, &times);
        assert_eq!(rows.len(), 3);
        rows[1].item = Some("#580".to_owned());
        let first = &rows[0];
        assert_eq!(first.work, None);
        assert_eq!(first.unpriced, 1);
        let item = &rows[1];
        assert_eq!(
            item.model.as_deref(),
            Some("claude-haiku-5-5, claude-opus-5-5")
        );
        assert_eq!(item.usage.as_ref().unwrap().cost_usd, 0.75);
        assert_eq!(item.time.testing, Some(750_000));
        let table = table(&rows);
        let line_of = |needle: &str| {
            table
                .lines()
                .find(|line| line.contains(needle))
                .unwrap_or_default()
                .to_owned()
        };
        let named = line_of("#580");
        for expected in ["host", "$0.7500", "1h 07m", "12m 30s", "80.0%"] {
            assert!(named.contains(expected), "{expected} missing from {named}");
        }
        let unattributed = line_of(UNATTRIBUTED);
        assert!(unattributed.contains("+1 unpriced"), "{unattributed}");
        assert!(line_of("total").contains("+1 unpriced"), "{table}");
    }

    #[test]
    fn a_tasks_time_shows_and_adds_up_in_the_total() {
        let mut timed = row("a", None, Some(used(1, 0.1, 1, 1)), TaskState::Completed);
        timed.time = Time {
            work: Some(60_000),
            testing: Some(30_000),
            waiting: Some(5_000),
        };
        let mut older = row("b", None, Some(used(1, 0.1, 1, 1)), TaskState::Completed);
        older.time.waiting = Some(0);
        let table = table(&[timed, older]);
        let line_b = table.lines().find(|line| line.contains(" b ")).unwrap();
        assert!(line_b.contains("n/a"), "{line_b}");
        let total = table.lines().find(|line| line.contains("total")).unwrap();
        for expected in ["1m 00s", "30s", "5s"] {
            assert!(total.contains(expected), "{expected} missing from {total}");
        }
    }
}
