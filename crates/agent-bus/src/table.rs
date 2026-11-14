//! The table of `agent-bus agents`: one row per agent from `status::Row`, and the host's record's
//! lag under it. The record's own tables are in `report`.

use shared::util::{TableLayout, render_table};

use crate::agents::Role;
use crate::channel::RECORDER;
use crate::status::{RecordLag, Row, age};

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

/// The agents table, or a sentence when there are no agents. Under it, for the host, a line
/// with its record's lag: what the recorder's consumer has not had recorded, and when it last
/// delivered.
pub fn agents(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "No agents".to_owned();
    }
    let cells: Vec<[String; 7]> = rows.iter().map(agent_cells).collect();
    let mut text = render_table(
        AGENT_HEADERS,
        &cells,
        &TableLayout {
            right: &AGENT_NUMBER_COLUMNS,
            ..TableLayout::default()
        },
    );
    for row in rows.iter().filter(|row| row.role == Role::Host) {
        text.push('\n');
        text.push_str(&record_line(row.record.as_ref()));
    }
    text
}

/// The line under the agents table for the host's record.
fn record_line(lag: Option<&RecordLag>) -> String {
    let Some(lag) = lag else {
        return format!(
            "Record ({RECORDER}): no consumer on the bus; `agent-bus bus start` creates it"
        );
    };
    let messages = if lag.unread == 1 {
        "message"
    } else {
        "messages"
    };
    let delivered = match lag.last_delivered_age_secs {
        Some(secs) => format!("last delivered {} ago", age(secs)),
        None => "never delivered".to_owned(),
    };
    format!(
        "Record ({RECORDER}): {} {messages} not recorded yet, {delivered}",
        lag.unread
    )
}

fn agent_cells(row: &Row) -> [String; 7] {
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
    [
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
            record: None,
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
    fn the_record_s_lag_is_shown_under_the_table() {
        let host = Row {
            record: Some(RecordLag {
                unread: 3,
                last_delivered_ms: Some(1),
                last_delivered_age_secs: Some(125),
            }),
            ..row("host", Role::Host, Online::Yes, None)
        };
        let dev = row("dev", Role::Guest, Online::Yes, None);
        let table = agents(&[host.clone(), dev.clone()]);
        let last = table.lines().last().unwrap_or_default();
        assert_eq!(
            last,
            "Record (_record): 3 messages not recorded yet, last delivered 2m 05s ago"
        );
        // Beside the host's own inbox, whose unread count stays in the JSON.
        assert!(table.contains(" host "), "{table}");

        let idle = Row {
            record: Some(RecordLag {
                unread: 0,
                last_delivered_ms: None,
                last_delivered_age_secs: None,
            }),
            ..host.clone()
        };
        assert!(
            agents(&[idle])
                .ends_with("Record (_record): 0 messages not recorded yet, never delivered"),
        );
        let missing = Row {
            record: None,
            ..host
        };
        assert!(agents(&[missing]).ends_with(
            "Record (_record): no consumer on the bus; `agent-bus bus start` creates it"
        ),);
        // Without a host there is no record to speak of.
        assert!(!agents(&[dev]).contains("Record"));
    }

    #[test]
    fn no_agents_is_a_sentence() {
        assert_eq!(agents(&[]), "No agents");
    }
}
