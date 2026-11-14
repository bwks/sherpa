//! Work items in the record: applying what a message says about its task's work (`facts`),
//! `link`, and `work backfill`. Each runs in the caller's transaction or its own.
//!
//! The rules:
//!
//! - every ref a task's messages name is kept in `task_ref`;
//! - a task without an item gets the item of the first of its refs that names one, else a new
//!   item; it keeps that item whatever its later messages say;
//! - a ref that names no item yet names the task's item; one that names another item is left
//!   there, a conflict that `work show` lists;
//! - only `link` merges two items.

use anyhow::{Context, Result, bail};
use tracing::instrument;
use turso::{Connection, Row, Value};

use crate::store::cells::{Cells, integer};
use crate::store::facts::{Facts, Ref, Source, parse_generation_line};
use crate::store::sql::{
    FIND_REF_WORK, FIND_TASK, FIND_TASK_WORK, INSERT_TASK_REF, INSERT_TASK_WORK, INSERT_WORK_ITEM,
    INSERT_WORK_REF, ITEM_REFS, ITEM_TASKS, MERGE_WORK_ITEM, MOVE_HOST_FOCUS, MOVE_HOST_USAGE,
    MOVE_TASK_WORK, MOVE_WORK_REFS, UNREPORTED_OUTCOMES,
};

/// What applying one message's facts changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Applied {
    /// Refs the task had not named before.
    pub new_refs: u64,
    /// Whether the task was put in an item.
    pub assigned: bool,
    /// The task's item.
    pub item: i64,
}

/// What `link` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Linked {
    /// The task had no item and the ref named none: a new item, named by the ref.
    Created { item: i64 },
    /// The ref names the task's item now.
    Named { item: i64 },
    /// The task, which had no item, is in the ref's item.
    Assigned { item: i64 },
    /// The ref already named the task's item.
    Already { item: i64 },
    /// The task's item was merged into the ref's: its tasks, refs and host usage moved there.
    Merged {
        from: i64,
        into: i64,
        tasks: Vec<String>,
        refs: Vec<Ref>,
    },
}

/// What `work backfill` did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Backfilled {
    /// Outcomes without `ended` looked at.
    pub outcomes: u64,
    /// Those whose Generation line named a ref.
    pub named: u64,
    /// Refs tasks had not named before.
    pub new_refs: u64,
    /// Tasks put in an item.
    pub assigned: u64,
}

/// Apply `facts`, from the message at stream sequence `seq` recorded at `at_ms`, to `task`'s
/// work, on `conn`, within the caller's transaction.
///
/// # Errors
///
/// Fails when a statement fails.
#[instrument(level = "debug", skip(conn, facts), fields(source = facts.source.as_str()))]
pub async fn apply(
    conn: &Connection,
    task: &str,
    facts: &Facts,
    seq: u64,
    at_ms: u64,
) -> Result<Applied> {
    let source = facts.source.as_str();
    let mut applied = Applied::default();
    for named in &facts.refs {
        let inserted = conn
            .execute(
                INSERT_TASK_REF,
                vec![
                    Value::Text(task.to_owned()),
                    Value::Text(named.repo.clone()),
                    Value::Text(named.kind.as_str().to_owned()),
                    Value::Text(named.name.clone()),
                    Value::Text(source.to_owned()),
                    integer(seq),
                ],
            )
            .await
            .context("Unable to record a ref of the task")?;
        applied.new_refs += inserted;
    }
    let item = if let Some(item) = task_item(conn, task).await? {
        item
    } else {
        let mut owner = None;
        for named in &facts.refs {
            owner = ref_item(conn, named).await?;
            if owner.is_some() {
                break;
            }
        }
        let item = match owner {
            Some(item) => item,
            None => new_item(conn, at_ms).await?,
        };
        assign(conn, task, item, facts.source, at_ms).await?;
        applied.assigned = true;
        item
    };
    for named in &facts.refs {
        name_item(conn, named, item, facts.source, at_ms).await?;
    }
    applied.item = item;
    Ok(applied)
}

/// The item `target` names: a new one named by it, made at `at_ms`, when it names none.
///
/// # Errors
///
/// Fails when a statement fails.
#[instrument(level = "debug", skip(conn))]
pub async fn item_for(conn: &Connection, target: &Ref, at_ms: u64) -> Result<i64> {
    if let Some(item) = ref_item(conn, target).await? {
        return Ok(item);
    }
    let item = new_item(conn, at_ms).await?;
    name_item(conn, target, item, Source::Manual, at_ms).await?;
    Ok(item)
}

/// Put `task` in the item `target` names, as a person asked: a new item when neither the task
/// nor the ref has one, and the task's item merged into the ref's when they differ.
///
/// # Errors
///
/// Fails when the record has no such task, or a statement or the commit fails.
#[instrument(skip(conn))]
pub async fn link(conn: &mut Connection, task: &str, target: &Ref, at_ms: u64) -> Result<Linked> {
    let tx = conn
        .transaction()
        .await
        .context("Unable to start a transaction")?;
    let mut rows = tx
        .query(FIND_TASK, vec![Value::Text(task.to_owned())])
        .await
        .context("Unable to read the task")?;
    if rows
        .next()
        .await
        .context("Unable to read the task")?
        .is_none()
    {
        bail!("The record has no task '{task}'");
    }
    drop(rows);
    let linked = match (ref_item(&tx, target).await?, task_item(&tx, task).await?) {
        (None, None) => {
            let item = new_item(&tx, at_ms).await?;
            name_item(&tx, target, item, Source::Manual, at_ms).await?;
            assign(&tx, task, item, Source::Manual, at_ms).await?;
            Linked::Created { item }
        }
        (None, Some(item)) => {
            name_item(&tx, target, item, Source::Manual, at_ms).await?;
            Linked::Named { item }
        }
        (Some(item), None) => {
            assign(&tx, task, item, Source::Manual, at_ms).await?;
            Linked::Assigned { item }
        }
        (Some(owner), Some(current)) if owner == current => Linked::Already { item: owner },
        (Some(into), Some(from)) => {
            let tasks = item_task_ids(&tx, from).await?;
            let refs = item_refs(&tx, from).await?;
            for statement in [
                MERGE_WORK_ITEM,
                MOVE_WORK_REFS,
                MOVE_TASK_WORK,
                MOVE_HOST_USAGE,
                MOVE_HOST_FOCUS,
            ] {
                tx.execute(statement, vec![Value::Integer(from), Value::Integer(into)])
                    .await
                    .context("Unable to merge the items")?;
            }
            Linked::Merged {
                from,
                into,
                tasks,
                refs,
            }
        }
    };
    tx.commit().await.context("Unable to commit the link")?;
    Ok(linked)
}

/// Parse the Generation line of every recorded outcome without `ended` and apply what it names,
/// with `home_repo` the host's repository, in one transaction. Running it again changes nothing.
///
/// # Errors
///
/// Fails when a statement or the commit fails; nothing is changed then.
#[instrument(skip(conn))]
pub async fn backfill(conn: &mut Connection, home_repo: Option<&str>) -> Result<Backfilled> {
    let tx = conn
        .transaction()
        .await
        .context("Unable to start a transaction")?;
    let mut rows = tx
        .query(UNREPORTED_OUTCOMES, ())
        .await
        .context("Unable to read the outcomes")?;
    let mut outcomes = Vec::new();
    while let Some(row) = rows.next().await.context("Unable to read the outcomes")? {
        let mut cells = Cells::new(&row);
        outcomes.push((
            cells.count("seq")?,
            cells.count("at_ms")?,
            cells.text("task")?,
            cells.text("text")?,
        ));
    }
    drop(rows);
    let mut done = Backfilled::default();
    for (seq, at_ms, task, text) in outcomes {
        done.outcomes += 1;
        let refs = parse_generation_line(&text, home_repo);
        if refs.is_empty() {
            continue;
        }
        done.named += 1;
        let facts = Facts {
            source: Source::Parsed,
            refs,
        };
        let applied = apply(&tx, &task, &facts, seq, at_ms).await?;
        done.new_refs += applied.new_refs;
        done.assigned += u64::from(applied.assigned);
    }
    tx.commit().await.context("Unable to commit the backfill")?;
    Ok(done)
}

/// The item `named` names, if any.
///
/// # Errors
///
/// Fails when the query fails.
#[instrument(level = "debug", skip(conn))]
pub async fn ref_item(conn: &Connection, named: &Ref) -> Result<Option<i64>> {
    let mut rows = conn
        .query(
            FIND_REF_WORK,
            vec![
                Value::Text(named.repo.clone()),
                Value::Text(named.kind.as_str().to_owned()),
                Value::Text(named.name.clone()),
            ],
        )
        .await
        .context("Unable to read the item of a ref")?;
    first_integer(
        rows.next()
            .await
            .context("Unable to read the item of a ref")?,
    )
}

/// The item `task` is in, if any.
async fn task_item(conn: &Connection, task: &str) -> Result<Option<i64>> {
    let mut rows = conn
        .query(FIND_TASK_WORK, vec![Value::Text(task.to_owned())])
        .await
        .context("Unable to read the item of a task")?;
    first_integer(
        rows.next()
            .await
            .context("Unable to read the item of a task")?,
    )
}

/// The first column of `row`, an integer, when there is a row.
fn first_integer(row: Option<Row>) -> Result<Option<i64>> {
    row.map(|row| match row.get_value(0).context("No column")? {
        Value::Integer(value) => Ok(value),
        other => bail!("Not an item ID: {other:?}"),
    })
    .transpose()
}

/// A new item, made at `at_ms`.
async fn new_item(conn: &Connection, at_ms: u64) -> Result<i64> {
    conn.execute(INSERT_WORK_ITEM, vec![integer(at_ms)])
        .await
        .context("Unable to record a new item")?;
    Ok(conn.last_insert_rowid())
}

/// Put `task` in `item`, unless it is in one.
async fn assign(
    conn: &Connection,
    task: &str,
    item: i64,
    source: Source,
    at_ms: u64,
) -> Result<()> {
    conn.execute(
        INSERT_TASK_WORK,
        vec![
            Value::Text(task.to_owned()),
            Value::Integer(item),
            Value::Text(source.as_str().to_owned()),
            integer(at_ms),
        ],
    )
    .await
    .context("Unable to put the task in its item")?;
    Ok(())
}

/// Name `item` by `named`, unless `named` already names an item.
async fn name_item(
    conn: &Connection,
    named: &Ref,
    item: i64,
    source: Source,
    at_ms: u64,
) -> Result<()> {
    conn.execute(
        INSERT_WORK_REF,
        vec![
            Value::Text(named.repo.clone()),
            Value::Text(named.kind.as_str().to_owned()),
            Value::Text(named.name.clone()),
            Value::Integer(item),
            Value::Text(source.as_str().to_owned()),
            integer(at_ms),
        ],
    )
    .await
    .context("Unable to name the item")?;
    Ok(())
}

/// The tasks of `item`.
async fn item_task_ids(conn: &Connection, item: i64) -> Result<Vec<String>> {
    let mut rows = conn
        .query(ITEM_TASKS, vec![Value::Integer(item)])
        .await
        .context("Unable to read the tasks of an item")?;
    let mut tasks = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the tasks of an item")?
    {
        let mut cells = Cells::new(&row);
        cells.id("work")?;
        cells.text("source")?;
        tasks.push(cells.text("id")?);
    }
    Ok(tasks)
}

/// The refs of `item`.
async fn item_refs(conn: &Connection, item: i64) -> Result<Vec<Ref>> {
    let mut rows = conn
        .query(ITEM_REFS, vec![Value::Integer(item)])
        .await
        .context("Unable to read the refs of an item")?;
    let mut refs = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .context("Unable to read the refs of an item")?
    {
        let mut cells = Cells::new(&row);
        cells.id("work")?;
        refs.push(cells.reference()?);
    }
    Ok(refs)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::message::{Ended, Kind, Message, PullRequestRef, RefKind, Work, WorkRef};
    use crate::store::database::Store;
    use crate::store::items::{self, Item};
    use crate::store::task::Recorded;
    use crate::store::write::record;

    const REPO: &str = "o/r";
    const DOCS: &str = "o/r-docs";

    /// A record in memory, and the next stream sequence to record at.
    struct Bench {
        _store: Store,
        conn: Connection,
        seq: u64,
    }

    impl Bench {
        async fn new() -> Self {
            let store = Store::open_in_memory().await.unwrap();
            let conn = store.connect().unwrap();
            Self {
                _store: store,
                conn,
                seq: 0,
            }
        }

        async fn record(&mut self, message: &Message) {
            self.seq += 1;
            let recorded = Recorded {
                seq: self.seq,
                at_ms: self.seq * 1000,
                message,
            };
            assert!(record(&mut self.conn, &recorded, Some(REPO)).await.unwrap());
        }

        /// The item `named` names, read in full.
        async fn item(&self, named: &Ref) -> Item {
            let id = ref_item(&self.conn, named).await.unwrap().unwrap();
            items::item(&self.conn, id, None).await.unwrap()
        }

        async fn task_item(&self, task: &str) -> Option<i64> {
            task_item(&self.conn, task).await.unwrap()
        }

        async fn count(&self, table: &str) -> i64 {
            let mut rows = self
                .conn
                .query(&format!("SELECT count(*) FROM {table}"), ())
                .await
                .unwrap();
            let row = rows.next().await.unwrap().unwrap();
            *row.get_value(0).unwrap().as_integer().unwrap()
        }
    }

    fn task(id: &str, tags: &[(RefKind, u64)]) -> Message {
        Message {
            task: id.to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind: Kind::Task,
            text: format!("work on {id}"),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
            work: (!tags.is_empty()).then(|| Work {
                refs: tags
                    .iter()
                    .map(|(kind, number)| WorkRef {
                        repo: REPO.to_owned(),
                        kind: *kind,
                        number: *number,
                    })
                    .collect(),
                session: Some("host-session".to_owned()),
            }),
            ended: None,
        }
    }

    fn outcome(id: &str, text: &str, ended: Option<Ended>) -> Message {
        Message {
            task: id.to_owned(),
            from: "dev".to_owned(),
            to: "host".to_owned(),
            kind: Kind::Completed,
            text: text.to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
            work: None,
            ended,
        }
    }

    fn ended_on(branch: Option<&str>, pull_requests: &[(&str, u64)]) -> Ended {
        Ended {
            repo: REPO.to_owned(),
            branch: branch.map(str::to_owned),
            pull_requests: pull_requests
                .iter()
                .map(|(repo, number)| PullRequestRef {
                    repo: (*repo).to_owned(),
                    number: *number,
                    url: format!("https://github.com/{repo}/pull/{number}"),
                })
                .collect(),
        }
    }

    fn issue(number: u64) -> Ref {
        Ref::new(REPO, RefKind::Issue, number.to_string())
    }

    fn pull_request(repo: &str, number: u64) -> Ref {
        Ref::new(repo, RefKind::PullRequest, number.to_string())
    }

    fn branch(name: &str) -> Ref {
        Ref::new(REPO, RefKind::Branch, name)
    }

    fn refs(item: &Item) -> Vec<(Ref, &str)> {
        item.refs
            .iter()
            .map(|named| (named.reference.clone(), named.source.as_str()))
            .collect()
    }

    fn tasks(item: &Item) -> Vec<&str> {
        item.tasks.iter().map(|task| task.task.as_str()).collect()
    }

    #[tokio::test]
    async fn a_tagged_task_and_its_guest_report_make_one_item_across_repositories() {
        let mut bench = Bench::new().await;
        bench.record(&task("t1", &[(RefKind::Issue, 580)])).await;
        let item = bench.item(&issue(580)).await;
        assert_eq!(item.name, "#580");
        assert_eq!(tasks(&item), ["t1"]);
        assert_eq!(item.tasks[0].source, "brief");

        bench
            .record(&outcome(
                "t1",
                "done",
                Some(ended_on(Some("feat/x"), &[(REPO, 612), (DOCS, 3)])),
            ))
            .await;
        let item = bench.item(&issue(580)).await;
        assert_eq!(
            refs(&item),
            [
                (branch("feat/x"), "guest"),
                (issue(580), "brief"),
                (pull_request(DOCS, 3), "guest"),
                (pull_request(REPO, 612), "guest"),
            ]
        );
        // The docs pull request on the same branch is a name of the same item.
        assert_eq!(bench.item(&pull_request(DOCS, 3)).await.id, item.id);
        assert_eq!(item.conflicts, []);

        // A later untagged task that ends on the branch joins the item.
        bench.record(&task("t2", &[])).await;
        assert_eq!(bench.task_item("t2").await, None);
        bench
            .record(&outcome("t2", "done", Some(ended_on(Some("feat/x"), &[]))))
            .await;
        let item = bench.item(&branch("feat/x")).await;
        assert_eq!(tasks(&item), ["t1", "t2"]);
        assert_eq!(item.tasks[1].source, "guest");
        assert_eq!(bench.count("work_item").await, 1);
    }

    #[tokio::test]
    async fn an_untagged_task_on_a_new_branch_gets_a_new_item_named_by_it() {
        let mut bench = Bench::new().await;
        bench.record(&task("t1", &[])).await;
        bench
            .record(&outcome("t1", "done", Some(ended_on(Some("fix/y"), &[]))))
            .await;
        let item = bench.item(&branch("fix/y")).await;
        assert_eq!(item.name, "fix/y");
        assert_eq!(tasks(&item), ["t1"]);
    }

    #[tokio::test]
    async fn the_default_branch_is_never_a_ref() {
        let mut bench = Bench::new().await;
        for id in ["t1", "t2"] {
            bench.record(&task(id, &[])).await;
            // On main the guest reports no branch, and no pull request.
            bench
                .record(&outcome(id, "done", Some(ended_on(None, &[]))))
                .await;
            assert_eq!(bench.task_item(id).await, None, "{id}");
        }
        assert_eq!(bench.count("work_item").await, 0);
        assert_eq!(bench.count("work_ref").await, 0);
        assert_eq!(bench.count("task_ref").await, 0);
    }

    #[tokio::test]
    async fn a_ref_naming_another_item_is_kept_as_a_conflict_and_not_merged() {
        let mut bench = Bench::new().await;
        bench.record(&task("t1", &[(RefKind::Issue, 1)])).await;
        bench
            .record(&outcome(
                "t1",
                "done",
                Some(ended_on(Some("feat/one"), &[])),
            ))
            .await;
        bench.record(&task("t2", &[(RefKind::Issue, 2)])).await;
        // t2 ends on t1's branch: the branch stays t1's item, and t2 stays in its own.
        bench
            .record(&outcome(
                "t2",
                "done",
                Some(ended_on(Some("feat/one"), &[(REPO, 9)])),
            ))
            .await;
        let one = bench.item(&issue(1)).await;
        let two = bench.item(&issue(2)).await;
        assert_ne!(one.id, two.id);
        assert_eq!(tasks(&one), ["t1"]);
        assert_eq!(tasks(&two), ["t2"]);
        // The free pull request went to t2's item.
        assert_eq!(bench.item(&pull_request(REPO, 9)).await.id, two.id);
        // Both items show the conflict.
        for item in [&one, &two] {
            assert_eq!(item.conflicts.len(), 1, "{item:?}");
            let conflict = &item.conflicts[0];
            assert_eq!(conflict.task, "t2");
            assert_eq!(conflict.reference, branch("feat/one"));
            assert_eq!(conflict.source, "guest");
            assert_eq!(
                (
                    conflict.task_item_name.as_str(),
                    conflict.ref_item_name.as_str()
                ),
                ("#2", "#1")
            );
        }
    }

    #[tokio::test]
    async fn link_merges_the_task_s_item_into_the_ref_s_and_keeps_the_trace() {
        let mut bench = Bench::new().await;
        bench.record(&task("t1", &[(RefKind::Issue, 1)])).await;
        bench.record(&task("t2", &[(RefKind::Issue, 2)])).await;
        bench
            .record(&outcome(
                "t2",
                "done",
                Some(ended_on(Some("feat/two"), &[])),
            ))
            .await;
        let one = bench.item(&issue(1)).await.id;
        let two = bench.item(&issue(2)).await.id;

        let linked = link(&mut bench.conn, "t2", &issue(1), 99_000)
            .await
            .unwrap();
        assert_eq!(
            linked,
            Linked::Merged {
                from: two,
                into: one,
                tasks: vec!["t2".to_owned()],
                refs: vec![branch("feat/two"), issue(2)],
            }
        );
        let merged = bench.item(&issue(2)).await;
        assert_eq!(merged.id, one);
        assert_eq!(merged.name, "#1");
        assert_eq!(tasks(&merged), ["t1", "t2"]);
        assert_eq!(merged.merged, [two]);
        // Linking again changes nothing.
        assert_eq!(
            link(&mut bench.conn, "t2", &issue(1), 99_000)
                .await
                .unwrap(),
            Linked::Already { item: one }
        );
    }

    #[tokio::test]
    async fn link_names_assigns_or_creates() {
        let mut bench = Bench::new().await;
        bench.record(&task("t1", &[(RefKind::Issue, 1)])).await;
        bench.record(&task("t2", &[])).await;
        bench.record(&task("t3", &[])).await;
        let one = bench.item(&issue(1)).await.id;
        // A free ref names the task's item.
        assert_eq!(
            link(&mut bench.conn, "t1", &branch("feat/one"), 1)
                .await
                .unwrap(),
            Linked::Named { item: one }
        );
        assert_eq!(bench.item(&branch("feat/one")).await.id, one);
        // A task without an item joins the ref's.
        assert_eq!(
            link(&mut bench.conn, "t2", &branch("feat/one"), 1)
                .await
                .unwrap(),
            Linked::Assigned { item: one }
        );
        // Neither has one: a new item.
        let Linked::Created { item } = link(&mut bench.conn, "t3", &issue(3), 1).await.unwrap()
        else {
            panic!("not created");
        };
        let three = bench.item(&issue(3)).await;
        assert_eq!((three.id, tasks(&three)), (item, vec!["t3"]));
        assert_eq!(refs(&three), [(issue(3), "manual")]);
        let error = link(&mut bench.conn, "nope", &issue(3), 1)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no task 'nope'"), "{error}");
    }

    /// The three Generation lines an older guest wrote, one task each.
    fn old_outcomes() -> [Message; 3] {
        [
            outcome(
                "t1",
                "Generation section for branch feat/x: updated https://github.com/o/r/pull/7; \
                 up to date https://github.com/o/r-docs/pull/3\n\ndone",
                None,
            ),
            outcome(
                "t2",
                "Generation section: no open pull request for branch fix/y\n\ndone",
                None,
            ),
            outcome(
                "t3",
                "Generation section not updated: the checkout is on a detached HEAD\n\ndone",
                None,
            ),
        ]
    }

    #[tokio::test]
    async fn catching_up_parses_the_generation_line_of_an_outcome_without_ended() {
        let mut bench = Bench::new().await;
        for message in old_outcomes() {
            bench.record(&message).await;
        }
        let x = bench.item(&branch("feat/x")).await;
        // No issue: the lowest pull request, in either repository.
        assert_eq!(x.name, "PR #3");
        assert_eq!(
            refs(&x),
            [
                (branch("feat/x"), "parsed"),
                (pull_request(DOCS, 3), "parsed"),
                (pull_request(REPO, 7), "parsed"),
            ]
        );
        assert_eq!(tasks(&bench.item(&branch("fix/y")).await), ["t2"]);
        assert_eq!(bench.task_item("t3").await, None);
    }

    /// A record holding the old outcomes as a record from before work items has them: the
    /// messages only, nothing derived.
    async fn recorded_before_work_items() -> Bench {
        let mut bench = Bench::new().await;
        for message in old_outcomes() {
            bench.record(&message).await;
        }
        for table in ["task_work", "task_ref", "work_ref", "work_item"] {
            bench
                .conn
                .execute(&format!("DELETE FROM {table}"), ())
                .await
                .unwrap();
        }
        bench
    }

    #[tokio::test]
    async fn backfill_parses_each_generation_line_once_however_often_it_runs() {
        let mut bench = recorded_before_work_items().await;
        let first = backfill(&mut bench.conn, Some(REPO)).await.unwrap();
        assert_eq!(
            first,
            Backfilled {
                outcomes: 3,
                named: 2,
                new_refs: 4,
                assigned: 2,
            }
        );
        assert_eq!(tasks(&bench.item(&pull_request(DOCS, 3)).await), ["t1"]);
        assert_eq!(tasks(&bench.item(&branch("fix/y")).await), ["t2"]);
        let counts = [
            bench.count("work_item").await,
            bench.count("work_ref").await,
            bench.count("task_ref").await,
            bench.count("task_work").await,
        ];
        let second = backfill(&mut bench.conn, Some(REPO)).await.unwrap();
        assert_eq!(
            second,
            Backfilled {
                outcomes: 3,
                named: 2,
                new_refs: 0,
                assigned: 0,
            }
        );
        assert_eq!(
            [
                bench.count("work_item").await,
                bench.count("work_ref").await,
                bench.count("task_ref").await,
                bench.count("task_work").await,
            ],
            counts
        );
    }

    #[tokio::test]
    async fn backfill_leaves_outcomes_with_ended_alone() {
        let mut bench = Bench::new().await;
        bench
            .record(&outcome(
                "t1",
                "Generation section for branch feat/x: updated https://github.com/o/r/pull/7",
                Some(ended_on(None, &[])),
            ))
            .await;
        let done = backfill(&mut bench.conn, Some(REPO)).await.unwrap();
        assert_eq!(done, Backfilled::default());
        assert_eq!(bench.count("work_ref").await, 0);
    }
}
