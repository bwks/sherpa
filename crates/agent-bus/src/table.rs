//! The tables agent-bus prints. `agents` shows one row per agent from `status::Row`.

use shared::util::render_modern_table;

use crate::agents::Role;
use crate::status::{Row, age};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::{Online, RunningRow};

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
}
