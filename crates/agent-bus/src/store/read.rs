//! What the commands read from the record: `usage`'s rows, a task's messages for `show`, and
//! the task list of `history`.

use anyhow::{Context, Result};
use serde::Serialize;
use tracing::instrument;
use turso::{Connection, Value};

use crate::message::{Kind, Message};
use crate::report::Row;
use crate::store::cells::{Cells, integer, optional_integer, optional_text};
use crate::store::sql::{HISTORY, TASK_MESSAGES, USAGE};
use crate::store::task::TaskState;

/// One task in `history`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryRow {
    pub task: String,
    pub agent: String,
    pub state: TaskState,
    /// The model it ran on, else the one it asked for; `None` for the guest's default.
    pub model: Option<String>,
    pub github: bool,
    /// The task's text; `None` when the record never saw the task message.
    pub prompt: Option<String>,
    pub sent_at_ms: Option<u64>,
    /// Its latest message.
    pub last_at_ms: u64,
    pub runs: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// What `usage` reports: the tasks `requester` sent that have an outcome, for `agent` only when
/// given, active at or after `since_ms` when given, in the order they were first recorded.
/// Tasks with no usage are left out unless `all`.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn usage(
    conn: &Connection,
    requester: &str,
    agent: Option<&str>,
    since_ms: Option<u64>,
    all: bool,
) -> Result<Vec<Row>> {
    let mut rows = conn
        .query(
            USAGE,
            vec![
                Value::Text(requester.to_owned()),
                optional_text(agent),
                optional_integer(since_ms),
                Value::Integer(i64::from(all)),
            ],
        )
        .await
        .context("Unable to read the usage")?;
    let mut found = Vec::new();
    while let Some(row) = rows.next().await.context("Unable to read the usage")? {
        found.push(usage_row(&mut Cells::new(&row)).context("A usage row is not one")?);
    }
    Ok(found)
}

/// The messages of `task` that `agent` sent or received, oldest first.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn task_messages(conn: &Connection, task: &str, agent: &str) -> Result<Vec<Message>> {
    let mut rows = conn
        .query(
            TASK_MESSAGES,
            vec![Value::Text(task.to_owned()), Value::Text(agent.to_owned())],
        )
        .await
        .context("Unable to read the task's messages")?;
    let mut found = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the task's messages")?
    {
        found.push(message_row(&mut Cells::new(&row)).context("A message row is not one")?);
    }
    Ok(found)
}

/// The newest `limit` tasks, for `agent` only when given, active at or after `since_ms` when
/// given, newest first.
///
/// # Errors
///
/// Fails when the query fails or a row is not one.
#[instrument(level = "debug", skip(conn))]
pub async fn history(
    conn: &Connection,
    agent: Option<&str>,
    since_ms: Option<u64>,
    limit: u64,
) -> Result<Vec<HistoryRow>> {
    let mut rows = conn
        .query(
            HISTORY,
            vec![
                optional_text(agent),
                optional_integer(since_ms),
                integer(limit),
            ],
        )
        .await
        .context("Unable to read the history")?;
    let mut found = Vec::new();
    while let Some(row) = rows.next().await.context("Unable to read the history")? {
        found.push(history_row(&mut Cells::new(&row)).context("A history row is not one")?);
    }
    Ok(found)
}

/// A row from the columns `usage.sql` selects.
fn usage_row(cells: &mut Cells<'_>) -> Result<Row> {
    Ok(Row {
        task: cells.text("id")?,
        agent: cells.text("agent")?,
        model: cells.optional_text("model")?,
        state: TaskState::parse(&cells.text("state")?)?,
        usage: cells.task_usage()?,
    })
}

/// A message from the columns `task_messages.sql` selects.
fn message_row(cells: &mut Cells<'_>) -> Result<Message> {
    let task = cells.text("task")?;
    let from = cells.text("from_agent")?;
    let to = cells.text("to_agent")?;
    let kind: Kind = serde_json::from_value(serde_json::Value::String(cells.text("kind")?))
        .context("kind is not a message kind")?;
    let text = cells.text("text")?;
    let model = cells.optional_text("model")?;
    let github = cells.boolean("github")?;
    let usage = cells
        .optional_text("usage")?
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .context("usage is not a run's usage")?;
    let task_usage = cells
        .optional_text("task_usage")?
        .map(|json| serde_json::from_str(&json))
        .transpose()
        .context("task_usage is not a task's usage")?;
    Ok(Message {
        task,
        from,
        to,
        kind,
        text,
        usage,
        task_usage,
        github,
        model,
    })
}

/// A row from the columns `history.sql` selects.
fn history_row(cells: &mut Cells<'_>) -> Result<HistoryRow> {
    let task = cells.text("id")?;
    let agent = cells.text("agent")?;
    let state = TaskState::parse(&cells.text("state")?)?;
    let requested_model = cells.optional_text("requested_model")?;
    let model = cells.optional_text("model")?.or(requested_model);
    Ok(HistoryRow {
        task,
        agent,
        state,
        model,
        github: cells.boolean("github")?,
        prompt: cells.optional_text("prompt")?,
        sent_at_ms: cells.optional_count("sent_at_ms")?,
        last_at_ms: cells.count("last_at_ms")?,
        runs: cells.optional_count("runs")?,
        cost_usd: cells.optional_real("cost_usd")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::message::TaskUsage;
    use crate::store::database::Store;
    use crate::store::task::Recorded;
    use crate::store::write::record;

    fn message(task: &str, from: &str, to: &str, kind: Kind) -> Message {
        Message {
            task: task.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            kind,
            text: format!("{} of {task}", kind.as_str()),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
        }
    }

    fn outcome(task: &str, agent: &str, kind: Kind, usage: Option<(u64, f64)>) -> Message {
        let mut message = message(task, agent, "host", kind);
        message.task_usage = usage.map(|(runs, cost_usd)| TaskUsage {
            runs,
            cost_usd,
            input: 10,
            output: 5,
            cache_read: 90,
            ..TaskUsage::default()
        });
        message
    }

    /// A record holding `messages`, the first at sequence 1 and time 1 s, one second apart.
    async fn recorded(messages: &[Message]) -> Connection {
        let store = Store::open_in_memory().await.unwrap();
        let mut conn = store.connect().unwrap();
        for (seq, message) in (1_u64..).zip(messages) {
            record(
                &mut conn,
                &Recorded {
                    seq,
                    at_ms: seq * 1000,
                    message,
                },
            )
            .await
            .unwrap();
        }
        conn
    }

    #[tokio::test]
    async fn usage_has_one_row_per_task_with_its_latest_totals_and_state() {
        let conn = recorded(&[
            message("a", "host", "dev", Kind::Task),
            outcome("a", "dev", Kind::Progress, None),
            outcome("a", "dev", Kind::Question, Some((1, 0.04))),
            outcome("b", "test", Kind::Failed, None),
            outcome("a", "dev", Kind::Completed, Some((2, 0.06))),
            // Sent, and nothing back yet: not in the report.
            message("c", "host", "dev", Kind::Task),
        ])
        .await;
        let rows = usage(&conn, "host", None, None, true).await.unwrap();
        let tasks: Vec<(&str, TaskState)> = rows
            .iter()
            .map(|row| (row.task.as_str(), row.state))
            .collect();
        assert_eq!(
            tasks,
            [("a", TaskState::Completed), ("b", TaskState::Failed)]
        );
        assert_eq!(rows[0].usage.as_ref().map(|usage| usage.runs), Some(2));
        assert_eq!(rows[1].usage, None);

        // Without `all`, only tasks with usage.
        let rows = usage(&conn, "host", None, None, false).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].task, "a");
    }

    #[tokio::test]
    async fn usage_filters_by_agent_requester_and_time() {
        let conn = recorded(&[
            outcome("a", "dev", Kind::Completed, Some((1, 0.04))),
            outcome("b", "test", Kind::Completed, Some((1, 0.04))),
            outcome("c", "dev", Kind::Completed, Some((1, 0.04))),
        ])
        .await;
        let only = |rows: Vec<Row>| rows.into_iter().map(|row| row.task).collect::<Vec<_>>();
        assert_eq!(
            only(
                usage(&conn, "host", Some("test"), None, false)
                    .await
                    .unwrap()
            ),
            ["b"]
        );
        assert_eq!(
            only(usage(&conn, "host", None, Some(2000), false).await.unwrap()),
            ["b", "c"]
        );
        assert_eq!(
            usage(&conn, "other", None, None, true).await.unwrap(),
            Vec::<Row>::new()
        );
    }

    #[tokio::test]
    async fn a_task_s_messages_are_every_kind_both_ways_in_order() {
        let mut done = outcome("a", "dev", Kind::Completed, Some((1, 0.5)));
        done.model = Some("claude-opus-5-5".to_owned());
        let sent = [
            message("a", "host", "dev", Kind::Task),
            outcome("a", "dev", Kind::Progress, None),
            outcome("b", "dev", Kind::Completed, None),
            outcome("a", "dev", Kind::Question, None),
            message("a", "host", "dev", Kind::Answer),
            done.clone(),
        ];
        let conn = recorded(&sent).await;
        let shown = task_messages(&conn, "a", "host").await.unwrap();
        let kinds: Vec<Kind> = shown.iter().map(|message| message.kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::Task,
                Kind::Progress,
                Kind::Question,
                Kind::Answer,
                Kind::Completed
            ]
        );
        // A message reads back as it was sent.
        assert_eq!(shown.last(), Some(&done));
        assert_eq!(
            task_messages(&conn, "zzz", "host").await.unwrap(),
            Vec::<Message>::new()
        );
    }

    #[tokio::test]
    async fn history_lists_the_newest_tasks_first_with_their_prompt() {
        let mut task = message("a", "host", "dev", Kind::Task);
        task.model = Some("claude-haiku-5-5".to_owned());
        task.github = true;
        let conn = recorded(&[
            task,
            message("b", "host", "test", Kind::Task),
            outcome("a", "dev", Kind::Completed, Some((1, 0.25))),
            // Seen first through its outcome, older than the stream.
            outcome("c", "dev", Kind::Failed, None),
        ])
        .await;
        let rows = history(&conn, None, None, 10).await.unwrap();
        let tasks: Vec<&str> = rows.iter().map(|row| row.task.as_str()).collect();
        assert_eq!(tasks, ["c", "b", "a"]);
        let a = &rows[2];
        assert_eq!(a.state, TaskState::Completed);
        assert_eq!(a.model.as_deref(), Some("claude-haiku-5-5"));
        assert!(a.github);
        assert_eq!(a.prompt.as_deref(), Some("task of a"));
        assert_eq!(a.sent_at_ms, Some(1000));
        assert_eq!(a.last_at_ms, 3000);
        assert_eq!(a.runs, Some(1));
        assert_eq!(rows[0].prompt, None);

        let tasks =
            |rows: Vec<HistoryRow>| rows.into_iter().map(|row| row.task).collect::<Vec<_>>();
        assert_eq!(tasks(history(&conn, None, None, 1).await.unwrap()), ["c"]);
        assert_eq!(
            tasks(history(&conn, Some("test"), None, 10).await.unwrap()),
            ["b"]
        );
        assert_eq!(
            tasks(history(&conn, None, Some(3000), 10).await.unwrap()),
            ["c", "a"]
        );
    }
}
