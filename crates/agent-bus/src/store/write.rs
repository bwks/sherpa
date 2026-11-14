//! Recording one message: the `message` row, the task's row, the run with its models when
//! the message carries a run's usage, and what it says about its task's work item, in one
//! transaction.

use anyhow::{Context, Result};
use tracing::instrument;
use turso::{Connection, Value};

use crate::store::cells::{Cells, boolean, integer, optional_integer, optional_text};
use crate::store::facts::facts;
use crate::store::sql::{FIND_TASK, INSERT_MESSAGE, INSERT_RUN, INSERT_RUN_MODEL, UPSERT_TASK};
use crate::store::task::{self, Recorded, Task, TaskState};
use crate::store::work::apply;

/// Record `recorded` in one transaction, with `home_repo` the host's repository for a parsed
/// Generation line's branch. Returns `false`, changing nothing, when its sequence is already
/// recorded: a message the bus delivered again, after a command stopped between committing it
/// and acknowledging it.
///
/// # Errors
///
/// Fails when a statement or the commit fails; nothing is recorded then.
#[instrument(level = "debug", skip_all, fields(seq = recorded.seq, task = %recorded.message.task))]
pub async fn record(
    conn: &mut Connection,
    recorded: &Recorded<'_>,
    home_repo: Option<&str>,
) -> Result<bool> {
    let tx = conn
        .transaction()
        .await
        .context("Unable to start a transaction")?;
    let message = recorded.message;
    let inserted = tx
        .execute(INSERT_MESSAGE, message_values(recorded)?)
        .await
        .context("Unable to record the message")?;
    if inserted == 0 {
        return Ok(false);
    }
    let task = find_task(&tx, &message.task).await?;
    let task = task::apply(task, recorded);
    tx.execute(UPSERT_TASK, task_values(&task))
        .await
        .context("Unable to record the task")?;
    if let Some(facts) = facts(message, home_repo) {
        apply(&tx, &message.task, &facts, recorded.seq, recorded.at_ms)
            .await
            .context("Unable to record the task's work item")?;
    }
    if let Some(usage) = &message.usage
        && message.kind.sent_by_worker()
    {
        tx.execute(
            INSERT_RUN,
            vec![
                integer(recorded.seq),
                Value::Text(message.task.clone()),
                Value::Text(message.kind.as_str().to_owned()),
                optional_text(message.model.as_deref()),
                integer(usage.input),
                integer(usage.output),
                integer(usage.cache_creation),
                integer(usage.cache_read),
                integer(usage.num_turns),
                integer(usage.duration_ms),
                integer(usage.duration_api_ms),
                Value::Real(usage.cost_usd),
                Value::Real(usage.session_cost_usd),
            ],
        )
        .await
        .context("Unable to record the run")?;
        for model in &usage.models {
            tx.execute(
                INSERT_RUN_MODEL,
                vec![
                    integer(recorded.seq),
                    Value::Text(model.model.clone()),
                    Value::Text(model.canonical_model.clone()),
                    Value::Text(model.cost_basis.clone()),
                    integer(model.input),
                    integer(model.output),
                    integer(model.cache_creation),
                    integer(model.cache_read),
                    Value::Real(model.cost_usd),
                ],
            )
            .await
            .context("Unable to record a model of the run")?;
        }
    }
    tx.commit().await.context("Unable to commit the message")?;
    Ok(true)
}

/// The row of the task `id`, if it has one.
async fn find_task(conn: &Connection, id: &str) -> Result<Option<Task>> {
    let mut rows = conn
        .query(FIND_TASK, vec![Value::Text(id.to_owned())])
        .await
        .context("Unable to read the task")?;
    match rows.next().await.context("Unable to read the task")? {
        Some(row) => Ok(Some(
            task_from_row(&mut Cells::new(&row))
                .with_context(|| format!("The row of task '{id}' is not one"))?,
        )),
        None => Ok(None),
    }
}

/// A task from the columns `find_task.sql` selects.
fn task_from_row(cells: &mut Cells<'_>) -> Result<Task> {
    let mut task = Task {
        id: cells.text("id")?,
        agent: cells.text("agent")?,
        requester: cells.text("requester")?,
        first_seq: cells.count("first_seq")?,
        last_seq: cells.count("last_seq")?,
        last_at_ms: cells.count("last_at_ms")?,
        requested_model: cells.optional_text("requested_model")?,
        github: cells.boolean("github")?,
        prompt: cells.optional_text("prompt")?,
        sent_at_ms: cells.optional_count("sent_at_ms")?,
        started_at_ms: cells.optional_count("started_at_ms")?,
        finished_at_ms: cells.optional_count("finished_at_ms")?,
        state: TaskState::parse(&cells.text("state")?)?,
        model: cells.optional_text("model")?,
        result: cells.optional_text("result")?,
        questions: cells.count("questions")?,
        answers: cells.count("answers")?,
        usage: None,
    };
    task.usage = cells.task_usage()?;
    Ok(task)
}

/// The values of `insert_message.sql`.
fn message_values(recorded: &Recorded<'_>) -> Result<Vec<Value>> {
    let message = recorded.message;
    let usage = message
        .usage
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("Unable to encode the run's usage")?;
    let task_usage = message
        .task_usage
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("Unable to encode the task's usage")?;
    let work = message
        .work
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("Unable to encode the task's work")?;
    let ended = message
        .ended
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .context("Unable to encode where the run ended")?;
    Ok(vec![
        integer(recorded.seq),
        integer(recorded.at_ms),
        Value::Text(message.task.clone()),
        Value::Text(message.from.clone()),
        Value::Text(message.to.clone()),
        Value::Text(message.kind.as_str().to_owned()),
        Value::Text(message.text.clone()),
        optional_text(message.model.as_deref()),
        boolean(message.github),
        optional_text(usage.as_deref()),
        optional_text(task_usage.as_deref()),
        optional_text(work.as_deref()),
        optional_text(ended.as_deref()),
    ])
}

/// The values of `upsert_task.sql`.
fn task_values(task: &Task) -> Vec<Value> {
    let usage = task.usage.as_ref();
    vec![
        Value::Text(task.id.clone()),
        Value::Text(task.agent.clone()),
        Value::Text(task.requester.clone()),
        integer(task.first_seq),
        integer(task.last_seq),
        integer(task.last_at_ms),
        optional_text(task.requested_model.as_deref()),
        boolean(task.github),
        optional_text(task.prompt.as_deref()),
        optional_integer(task.sent_at_ms),
        optional_integer(task.started_at_ms),
        optional_integer(task.finished_at_ms),
        Value::Text(task.state.as_str().to_owned()),
        optional_text(task.model.as_deref()),
        optional_text(task.result.as_deref()),
        integer(task.questions),
        integer(task.answers),
        optional_integer(usage.map(|usage| usage.runs)),
        usage.map_or(Value::Null, |usage| Value::Real(usage.cost_usd)),
        optional_integer(usage.map(|usage| usage.input)),
        optional_integer(usage.map(|usage| usage.output)),
        optional_integer(usage.map(|usage| usage.cache_creation)),
        optional_integer(usage.map(|usage| usage.cache_read)),
        optional_integer(usage.map(|usage| usage.num_turns)),
        optional_integer(usage.map(|usage| usage.duration_ms)),
        optional_integer(usage.map(|usage| usage.duration_api_ms)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::message::{Kind, Message, ModelUsage, TaskUsage, Usage};
    use crate::store::database::Store;

    fn message(task: &str, kind: Kind) -> Message {
        let (from, to) = if kind.sent_by_worker() {
            ("dev", "host")
        } else {
            ("host", "dev")
        };
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
            work: None,
            ended: None,
        }
    }

    /// A completed outcome with a run on two models.
    fn completed(task: &str) -> Message {
        let mut done = message(task, Kind::Completed);
        done.model = Some("claude-opus-5-5".to_owned());
        done.usage = Some(Usage {
            input: 10,
            output: 20,
            cache_creation: 30,
            cache_read: 40,
            num_turns: 3,
            duration_ms: 5000,
            duration_api_ms: 4000,
            cost_usd: 0.75,
            session_cost_usd: 1.5,
            models: vec![
                ModelUsage {
                    model: "claude-opus-5-5".to_owned(),
                    cost_usd: 0.5,
                    canonical_model: "claude-opus-5-5".to_owned(),
                    cost_basis: "list".to_owned(),
                    ..ModelUsage::default()
                },
                ModelUsage {
                    model: "claude-haiku-5-5".to_owned(),
                    cost_usd: 0.25,
                    canonical_model: "claude-haiku-5-5".to_owned(),
                    cost_basis: "list".to_owned(),
                    ..ModelUsage::default()
                },
            ],
        });
        done.task_usage = Some(TaskUsage {
            runs: 2,
            cost_usd: 1.0,
            input: 11,
            ..TaskUsage::default()
        });
        done
    }

    async fn count(conn: &Connection, table: &str) -> i64 {
        let mut rows = conn
            .query(&format!("SELECT count(*) FROM {table}"), ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        *row.get_value(0).unwrap().as_integer().unwrap()
    }

    async fn record_at(conn: &mut Connection, seq: u64, message: &Message) -> bool {
        record(
            conn,
            &Recorded {
                seq,
                at_ms: seq * 1000,
                message,
            },
            None,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_task_s_messages_make_its_row_and_its_run() {
        let store = Store::open_in_memory().await.unwrap();
        let mut conn = store.connect().unwrap();
        assert!(record_at(&mut conn, 1, &message("t1", Kind::Task)).await);
        assert!(record_at(&mut conn, 2, &message("t1", Kind::Progress)).await);
        assert!(record_at(&mut conn, 3, &completed("t1")).await);

        assert_eq!(count(&conn, "message").await, 3);
        assert_eq!(count(&conn, "run").await, 1);
        assert_eq!(count(&conn, "run_model").await, 2);
        let task = find_task(&conn, "t1").await.unwrap().unwrap();
        assert_eq!(task.state, TaskState::Completed);
        assert_eq!(task.prompt.as_deref(), Some("task of t1"));
        assert_eq!(task.started_at_ms, Some(2000));
        assert_eq!(task.finished_at_ms, Some(3000));
        assert_eq!(task.model.as_deref(), Some("claude-opus-5-5"));
        let usage = task.usage.unwrap();
        assert_eq!((usage.runs, usage.input), (2, 11));
        assert!((usage.cost_usd - 1.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn a_sequence_recorded_again_changes_nothing() {
        let store = Store::open_in_memory().await.unwrap();
        let mut conn = store.connect().unwrap();
        let task = message("t1", Kind::Task);
        let done = completed("t1");
        assert!(record_at(&mut conn, 1, &task).await);
        assert!(record_at(&mut conn, 2, &done).await);
        let before = find_task(&conn, "t1").await.unwrap();

        // A command stopped after committing and before acknowledging: the bus delivers
        // both again.
        assert!(!record_at(&mut conn, 1, &task).await);
        assert!(!record_at(&mut conn, 2, &done).await);

        assert_eq!(count(&conn, "message").await, 2);
        assert_eq!(count(&conn, "run").await, 1);
        assert_eq!(count(&conn, "run_model").await, 2);
        assert_eq!(find_task(&conn, "t1").await.unwrap(), before);
    }

    #[tokio::test]
    async fn a_failed_record_leaves_nothing_behind() {
        let store = Store::open_in_memory().await.unwrap();
        let mut conn = store.connect().unwrap();
        // Without the `run_model` table the run's models cannot be written, after the message,
        // the task and the run were: the whole transaction is rolled back.
        conn.execute("DROP TABLE run_model", ()).await.unwrap();
        let error = record(
            &mut conn,
            &Recorded {
                seq: 1,
                at_ms: 1000,
                message: &completed("t1"),
            },
            None,
        )
        .await
        .err()
        .unwrap();
        assert!(format!("{error:#}").contains("model"), "{error:#}");
        assert_eq!(count(&conn, "message").await, 0);
        assert_eq!(count(&conn, "task").await, 0);
        assert_eq!(count(&conn, "run").await, 0);
    }

    #[tokio::test]
    async fn a_message_to_the_worker_with_usage_is_not_a_run() {
        let store = Store::open_in_memory().await.unwrap();
        let mut conn = store.connect().unwrap();
        let mut odd = message("t1", Kind::Answer);
        odd.usage = completed("t1").usage;
        assert!(record_at(&mut conn, 1, &odd).await);
        assert_eq!(count(&conn, "run").await, 0);
    }
}
