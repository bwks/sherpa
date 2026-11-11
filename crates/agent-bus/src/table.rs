//! The tables agent-bus prints. `agents` shows one row per agent from `status::Row`; `history`
//! one row per task from the record.

use jiff::Timestamp;
use shared::util::render_modern_table;

use crate::agents::Role;
use crate::status::{Row, age};
use crate::store::HistoryRow;

const AGENT_HEADERS: [&str; 7] = [
    "Agent",
    "Role",
    "Online",
    "Running task",
    "Running for",
    "Queued",
    "Last active",
];
/// The queued column holds a number, so it is right-aligned. It counts tasks and answers waiting
/// for a guest; the host has none (`-`): its unread messages are guests' progress and outcomes
/// waiting for a listener, and are in the JSON's `inbox_unread`.
const AGENT_NUMBER_COLUMNS: [usize; 1] = [5];

const HISTORY_HEADERS: [&str; 7] = [
    "Task",
    "Agent",
    "State",
    "Model",
    "Last active (UTC)",
    "Cost (not billed)",
    "Prompt",
];
/// The cost column is right-aligned.
const HISTORY_NUMBER_COLUMNS: [usize; 1] = [5];
/// How much of a task's prompt the history shows: the start of its first line.
const PROMPT_CHARS: usize = 60;

/// The agents table, or a sentence when there are no agents.
pub fn agents(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "No agents".to_owned();
    }
    let cells: Vec<Vec<String>> = rows.iter().map(agent_cells).collect();
    render_modern_table(&AGENT_HEADERS, &cells, &AGENT_NUMBER_COLUMNS)
}

fn agent_cells(row: &Row) -> Vec<String> {
    let none = || "-".to_owned();
    let (task, running_for) = match &row.running {
        Some(running) => (running.task.clone(), age(running.age_secs)),
        None => (none(), none()),
    };
    let role = match row.role {
        Role::Host => "host",
        Role::Guest => "guest",
    };
    let last_active = match row.last_active_age_secs {
        Some(secs) => format!("{} ago", age(secs)),
        None => "never".to_owned(),
    };
    vec![
        row.name.clone(),
        role.to_owned(),
        row.online.word().to_owned(),
        task,
        running_for,
        row.queued.map_or_else(none, |queued| queued.to_string()),
        last_active,
    ]
}

/// The history table, or a sentence when there are no tasks.
pub fn history(rows: &[HistoryRow]) -> String {
    if rows.is_empty() {
        return "No tasks recorded".to_owned();
    }
    let cells: Vec<Vec<String>> = rows.iter().map(history_cells).collect();
    render_modern_table(&HISTORY_HEADERS, &cells, &HISTORY_NUMBER_COLUMNS)
}

fn history_cells(row: &HistoryRow) -> Vec<String> {
    let none = || "-".to_owned();
    vec![
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
fn utc_minute(at_ms: u64) -> String {
    i64::try_from(at_ms)
        .ok()
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .map_or_else(
            || "?".to_owned(),
            |at| at.strftime("%Y-%m-%d %H:%M").to_string(),
        )
}

/// The start of `prompt`'s first line, with `…` when there is more.
fn prompt_start(prompt: &str) -> String {
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
    use crate::status::{Online, RunningRow};
    use crate::store::TaskState;

    fn row(name: &str, role: Role, online: Online, running: Option<RunningRow>) -> Row {
        Row {
            name: name.to_owned(),
            role,
            online,
            running,
            queued: match role {
                Role::Host => None,
                Role::Guest => Some(3),
            },
            inbox_unread: Some(1),
            local_queue: Some(2),
            reader_attached: true,
            last_active_ms: Some(1),
            last_active_age_secs: Some(42),
        }
    }

    #[test]
    fn each_agent_has_a_row_with_its_task_queue_and_activity() {
        let rows = [
            row("host", Role::Host, Online::Yes, None),
            row(
                "dev",
                Role::Guest,
                Online::Yes,
                Some(RunningRow {
                    task: "task-1".to_owned(),
                    age_secs: 185,
                }),
            ),
            Row {
                last_active_ms: None,
                last_active_age_secs: None,
                ..row("agent-2", Role::Guest, Online::No, None)
            },
        ];
        let table = agents(&rows);
        for header in AGENT_HEADERS {
            assert!(table.contains(header), "{header} missing from\n{table}");
        }
        let line = |name: &str| {
            table
                .lines()
                .find(|line| line.contains(&format!(" {name} ")))
                .unwrap_or_default()
                .to_owned()
        };
        let dev = line("dev");
        for cell in ["guest", "yes", "task-1", "3m 05s", "3", "42s ago"] {
            assert!(dev.contains(cell), "{cell} missing from {dev}");
        }
        // The host's unread messages are not tasks: its queued cell is `-`, not a count.
        let host = line("host");
        let cells: Vec<&str> = host.split('│').map(str::trim).collect();
        assert_eq!(cells.get(6), Some(&"-"), "{host}");
        let offline = line("agent-2");
        assert!(
            offline.contains("no") && offline.contains("never") && offline.contains(" - "),
            "{offline}"
        );
        // The modern style draws its box with these.
        assert!(table.starts_with('┌'), "{table}");
    }

    #[test]
    fn no_agents_is_a_sentence() {
        assert_eq!(agents(&[]), "No agents");
    }

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
        let table = history(&rows);
        for header in HISTORY_HEADERS {
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
        assert_eq!(history(&[]), "No tasks recorded");
    }
}
