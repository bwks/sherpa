//! `agent-bus usage --by agent|model|week`: the tasks of `usage` added up by group, with the
//! table it prints.
//!
//! By agent and by week, a task counts whole in one group: its agent's, or the ISO week (UTC)
//! it was sent in. By model, each run's per-model figures (`run_model`) go to their model, and
//! what the task's totals hold beyond its runs (the priming runs of its base session) goes to
//! the task's own model.

use std::collections::{BTreeMap, BTreeSet};

use clap::ValueEnum;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use shared::util::{TableLayout, render_table};

use crate::message::{TaskUsage, cache_hit_permille};
use crate::report::usage::Row;
use crate::store::items::RunPart;

/// The cost a remainder below which is rounding, not usage, in US dollars.
const COST_EPSILON: f64 = 1e-9;
/// The columns of numbers, which are right-aligned: all but the group.
const NUMBER_COLUMNS: [usize; 5] = [1, 2, 3, 4, 5];
/// The group of a run or a task with no model.
const UNKNOWN_MODEL: &str = "unknown";

/// What `usage --by` groups the tasks by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum By {
    /// The guest agent that ran the task.
    Agent,
    /// The model of each run; the task's model for its priming runs.
    Model,
    /// The ISO week the task was sent in, in UTC.
    Week,
}

/// One group's totals.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Group {
    pub key: String,
    pub tasks: u64,
    pub usage: TaskUsage,
}

/// `rows`, the tasks of `usage`, added up by `by`, with `runs` every run's parts for `By::Model`;
/// in key order. Tasks with no usage are left out.
pub fn group(rows: &[Row], runs: &[RunPart], by: By) -> Vec<Group> {
    let mut groups: BTreeMap<String, (BTreeSet<&str>, TaskUsage)> = BTreeMap::new();
    for row in rows {
        let Some(usage) = &row.usage else {
            continue;
        };
        let key = match by {
            By::Agent => row.agent.clone(),
            By::Week => iso_week(row.at_ms),
            By::Model => {
                let task_model = row.model.as_deref().unwrap_or(UNKNOWN_MODEL);
                let mut remainder = usage.clone();
                let mut seen = BTreeSet::new();
                for part in runs.iter().filter(|part| part.task == row.task) {
                    let key = part.model.as_deref().unwrap_or(task_model).to_owned();
                    let (tasks, sum) = groups.entry(key).or_default();
                    tasks.insert(&row.task);
                    add_part(sum, part);
                    if seen.insert(part.seq) {
                        remainder.runs = remainder.runs.saturating_sub(1);
                    }
                    remainder.cost_usd -= part.cost_usd;
                    remainder.input = remainder.input.saturating_sub(part.input);
                    remainder.output = remainder.output.saturating_sub(part.output);
                    remainder.cache_creation =
                        remainder.cache_creation.saturating_sub(part.cache_creation);
                    remainder.cache_read = remainder.cache_read.saturating_sub(part.cache_read);
                }
                // Each run counts once in each model it used.
                for part_seq in &seen {
                    let models: BTreeSet<&str> = runs
                        .iter()
                        .filter(|part| part.seq == *part_seq)
                        .map(|part| part.model.as_deref().unwrap_or(task_model))
                        .collect();
                    for model in models {
                        let (_, sum) = groups.entry(model.to_owned()).or_default();
                        sum.runs = sum.runs.saturating_add(1);
                    }
                }
                if remainder.runs == 0
                    && remainder.cost_usd.abs() < COST_EPSILON
                    && remainder.tokens() == 0
                {
                    continue;
                }
                let (tasks, sum) = groups.entry(task_model.to_owned()).or_default();
                tasks.insert(&row.task);
                add_totals(sum, &remainder);
                continue;
            }
        };
        let (tasks, sum) = groups.entry(key).or_default();
        tasks.insert(&row.task);
        add_totals(sum, usage);
    }
    groups
        .into_iter()
        .map(|(key, (tasks, usage))| Group {
            key,
            tasks: u64::try_from(tasks.len()).unwrap_or(u64::MAX),
            usage,
        })
        .collect()
}

/// The groups as a table with a total row, or a sentence when there are none.
pub fn table(groups: &[Group], by: By) -> String {
    if groups.is_empty() {
        return "No usage recorded".to_owned();
    }
    let first = match by {
        By::Agent => "Agent",
        By::Model => "Model",
        By::Week => "Week (UTC)",
    };
    let headers = [
        first,
        "Tasks",
        "Runs",
        "Cost (not billed)",
        "Cache hit",
        "Tokens",
    ];
    let mut total = TaskUsage::default();
    let mut tasks = 0_u64;
    let mut cells: Vec<[String; 6]> = groups
        .iter()
        .map(|group| {
            add_totals(&mut total, &group.usage);
            tasks = tasks.saturating_add(group.tasks);
            line(&group.key, group.tasks, &group.usage)
        })
        .collect();
    // By model, a task with runs on two models is in both groups: its count is not a sum.
    let total_tasks = if by == By::Model {
        String::new()
    } else {
        tasks.to_string()
    };
    let mut total_line = line("total", 0, &total);
    total_line[1] = total_tasks;
    cells.push(total_line);
    render_table(
        headers,
        &cells,
        &TableLayout {
            right: &NUMBER_COLUMNS,
            ..TableLayout::default()
        },
    )
}

/// One group's cells.
fn line(key: &str, tasks: u64, usage: &TaskUsage) -> [String; 6] {
    [
        key.to_owned(),
        tasks.to_string(),
        usage.runs.to_string(),
        format!("${:.4}", usage.cost_usd),
        cache_hit_permille(usage.input, usage.cache_creation, usage.cache_read).map_or_else(
            || "n/a".to_owned(),
            |permille| format!("{}.{}%", permille / 10, permille % 10),
        ),
        usage.tokens().to_string(),
    ]
}

/// Add `usage`'s runs, cost and tokens to `sum`.
fn add_totals(sum: &mut TaskUsage, usage: &TaskUsage) {
    sum.runs = sum.runs.saturating_add(usage.runs);
    sum.cost_usd += usage.cost_usd;
    sum.input = sum.input.saturating_add(usage.input);
    sum.output = sum.output.saturating_add(usage.output);
    sum.cache_creation = sum.cache_creation.saturating_add(usage.cache_creation);
    sum.cache_read = sum.cache_read.saturating_add(usage.cache_read);
}

/// Add one run part's cost and tokens to `sum`; its run is counted apart.
fn add_part(sum: &mut TaskUsage, part: &RunPart) {
    sum.cost_usd += part.cost_usd;
    sum.input = sum.input.saturating_add(part.input);
    sum.output = sum.output.saturating_add(part.output);
    sum.cache_creation = sum.cache_creation.saturating_add(part.cache_creation);
    sum.cache_read = sum.cache_read.saturating_add(part.cache_read);
}

/// The ISO week of `at_ms`, milliseconds since the Unix epoch, in UTC: `2026-W46`.
pub fn iso_week(at_ms: u64) -> String {
    i64::try_from(at_ms)
        .ok()
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .map_or_else(
            || "?".to_owned(),
            |at| {
                let week = at.to_zoned(TimeZone::UTC).date().iso_week_date();
                format!("{}-W{:02}", week.year(), week.week())
            },
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::store::TaskState;

    /// 2026-11-14 12:00 UTC, a Saturday of ISO week 46.
    const SATURDAY: u64 = 1_794_657_600_000;
    const DAY_MS: u64 = 86_400_000;

    fn row(task: &str, agent: &str, model: &str, at_ms: u64, cost_usd: f64, runs: u64) -> Row {
        Row {
            task: task.to_owned(),
            agent: agent.to_owned(),
            model: Some(model.to_owned()),
            state: TaskState::Completed,
            at_ms,
            usage: Some(TaskUsage {
                runs,
                cost_usd,
                input: 100,
                output: 10,
                ..TaskUsage::default()
            }),
        }
    }

    fn part(seq: u64, task: &str, model: Option<&str>, cost_usd: f64, input: u64) -> RunPart {
        RunPart {
            seq,
            task: task.to_owned(),
            model: model.map(str::to_owned),
            input,
            output: 0,
            cache_creation: 0,
            cache_read: 0,
            cost_usd,
        }
    }

    fn keys(groups: &[Group]) -> Vec<(&str, u64, u64)> {
        groups
            .iter()
            .map(|group| (group.key.as_str(), group.tasks, group.usage.runs))
            .collect()
    }

    #[test]
    fn weeks_are_iso_weeks_in_utc() {
        assert_eq!(iso_week(SATURDAY), "2026-W46");
        // Sunday 23:59:59 UTC is still the same week; Monday 00:00 UTC is the next.
        let monday = SATURDAY + 2 * DAY_MS - 12 * 3_600_000;
        assert_eq!(iso_week(monday - 1000), "2026-W46");
        assert_eq!(iso_week(monday), "2026-W47");
        // The first days of January can be in the last ISO week of the year before.
        assert_eq!(iso_week(1_798_761_600_000), "2026-W53");
        assert_eq!(iso_week(u64::MAX), "?");
    }

    #[test]
    fn by_agent_and_by_week_count_each_task_whole_in_one_group() {
        let rows = [
            row("a", "dev01", "opus", SATURDAY, 1.0, 2),
            row("b", "dev02", "opus", SATURDAY + 3 * DAY_MS, 0.5, 1),
            row("c", "dev01", "haiku", SATURDAY + 3 * DAY_MS, 0.25, 1),
        ];
        let by_agent = group(&rows, &[], By::Agent);
        assert_eq!(keys(&by_agent), [("dev01", 2, 3), ("dev02", 1, 1)]);
        assert!((by_agent[0].usage.cost_usd - 1.25).abs() < 1e-9);
        let by_week = group(&rows, &[], By::Week);
        assert_eq!(keys(&by_week), [("2026-W46", 1, 2), ("2026-W47", 2, 2)]);
        let table = table(&by_week, By::Week);
        assert!(table.contains("Week (UTC)"), "{table}");
        let total = table.lines().find(|line| line.contains("total")).unwrap();
        assert!(total.contains("$1.7500"), "{total}");
        assert!(total.contains(" 3 "), "{total}");
    }

    #[test]
    fn by_model_splits_runs_by_their_models_and_gives_the_rest_to_the_task_s_model() {
        // Task a: three runs counted, $1.00. Run 10 used opus ($0.60) and haiku ($0.10); run 11
        // named no model ($0.20). The rest, one priming run and $0.10, is the task's own model.
        let rows = [row("a", "dev", "opus", SATURDAY, 1.0, 3)];
        let runs = [
            part(10, "a", Some("opus"), 0.6, 50),
            part(10, "a", Some("haiku"), 0.1, 10),
            part(11, "a", None, 0.2, 20),
            part(12, "other", Some("sonnet"), 9.0, 1),
        ];
        let by_model = group(&rows, &runs, By::Model);
        assert_eq!(keys(&by_model), [("haiku", 1, 1), ("opus", 1, 3)]);
        let opus = &by_model[1].usage;
        assert!((opus.cost_usd - 0.9).abs() < 1e-9, "{opus:?}");
        assert_eq!(opus.input, 50 + 20 + 20);
        let total: f64 = by_model.iter().map(|group| group.usage.cost_usd).sum();
        assert!((total - 1.0).abs() < 1e-9);
        let table = table(&by_model, By::Model);
        assert!(table.contains("Model"), "{table}");
        assert!(table.contains("$1.0000"), "{table}");
    }

    #[test]
    fn a_task_without_usage_is_not_grouped() {
        let mut old = row("old", "dev", "opus", SATURDAY, 0.0, 0);
        old.usage = None;
        assert_eq!(group(&[old], &[], By::Agent), []);
        assert_eq!(table(&[], By::Agent), "No usage recorded");
    }
}
