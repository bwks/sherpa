//! What the work commands print: `work show` (one item, or every item), `open`, and the lines
//! of `link` and `work backfill`.

use std::fmt::Write as _;

use shared::util::{TableLayout, render_table};

use crate::report::history::{prompt_start, utc_minute};
use crate::store::items::{Item, ItemSummary, OpenTask};
use crate::store::{Backfilled, Linked, Ref};

/// The cost column of the tasks table is right-aligned.
const TASK_NUMBER_COLUMNS: [usize; 1] = [5];
/// The tasks, cost and age columns of the item list are right-aligned.
const ITEM_NUMBER_COLUMNS: [usize; 2] = [3, 4];
/// The age column of `open` is right-aligned.
const OPEN_NUMBER_COLUMNS: [usize; 1] = [4];
/// How much of a question `open` shows in its table; `--json` has all of it.
const QUESTION_CHARS: usize = 80;

/// One item: its refs, its tasks with their total cost, its conflicts and what was merged into
/// it.
pub fn item(item: &Item) -> String {
    let mut text = format!("Item {}: {}\n", item.id, item.name);
    let _ = writeln!(text, "{}", refs_table(item));
    let _ = writeln!(text, "{}", tasks_table(item));
    if let Some(conflicts) = conflicts_table(item) {
        let _ = writeln!(text, "{conflicts}");
    }
    if !item.merged.is_empty() {
        let merged: Vec<String> = item.merged.iter().map(|id| format!("item {id}")).collect();
        let _ = writeln!(text, "Merged into it: {}", merged.join(", "));
    }
    text.trim_end().to_owned()
}

/// The refs of `item`, or a sentence when it has none.
fn refs_table(item: &Item) -> String {
    let refs: Vec<[String; 3]> = item
        .refs
        .iter()
        .map(|named| {
            [
                named.reference.qualified(),
                named.source.clone(),
                utc_minute(named.at_ms),
            ]
        })
        .collect();
    if refs.is_empty() {
        return "No refs".to_owned();
    }
    render_table(
        ["Ref", "Source", "Since (UTC)"],
        &refs,
        &TableLayout {
            title: Some("Refs"),
            ..TableLayout::default()
        },
    )
}

/// The tasks of `item` with a total row, or a sentence when it has none.
fn tasks_table(item: &Item) -> String {
    if item.tasks.is_empty() {
        return "No tasks".to_owned();
    }
    let none = || "-".to_owned();
    let mut cost = 0.0;
    let mut tasks: Vec<[String; 7]> = item
        .tasks
        .iter()
        .map(|task| {
            cost += task.cost_usd.unwrap_or_default();
            [
                task.task.clone(),
                task.agent.clone(),
                task.state.as_str().to_owned(),
                task.model.clone().unwrap_or_else(|| "default".to_owned()),
                utc_minute(task.last_at_ms),
                task.cost_usd
                    .map_or_else(none, |cost| format!("${cost:.4}")),
                task.prompt.as_deref().map_or_else(none, prompt_start),
            ]
        })
        .collect();
    tasks.push([
        "total".to_owned(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        format!("${cost:.4}"),
        String::new(),
    ]);
    render_table(
        [
            "Task",
            "Agent",
            "State",
            "Model",
            "Last active (UTC)",
            "Cost (not billed)",
            "Prompt",
        ],
        &tasks,
        &TableLayout {
            title: Some("Tasks"),
            right: &TASK_NUMBER_COLUMNS,
            ..TableLayout::default()
        },
    )
}

/// The conflicts of `item`; `None` when it has none.
fn conflicts_table(item: &Item) -> Option<String> {
    if item.conflicts.is_empty() {
        return None;
    }
    let conflicts: Vec<[String; 4]> = item
        .conflicts
        .iter()
        .map(|conflict| {
            [
                conflict.task.clone(),
                conflict.reference.qualified(),
                format!("item {} ({})", conflict.task_item, conflict.task_item_name),
                format!("item {} ({})", conflict.ref_item, conflict.ref_item_name),
            ]
        })
        .collect();
    Some(render_table(
        ["Task", "Named", "Task's item", "Named item"],
        &conflicts,
        &TableLayout {
            title: Some("Conflicts: refs naming another item; `link` merges them"),
            ..TableLayout::default()
        },
    ))
}

/// Every item, as a table, or a sentence when there are none.
pub fn items(items: &[ItemSummary]) -> String {
    if items.is_empty() {
        return "No work items recorded".to_owned();
    }
    let cells: Vec<[String; 6]> = items
        .iter()
        .map(|item| {
            let refs: Vec<String> = item.refs.iter().map(Ref::label).collect();
            [
                item.id.to_string(),
                item.name.clone(),
                refs.join(", "),
                item.tasks.to_string(),
                format!("${:.4}", item.cost_usd),
                utc_minute(item.last_at_ms),
            ]
        })
        .collect();
    render_table(
        [
            "Item",
            "Name",
            "Refs",
            "Tasks",
            "Cost (not billed)",
            "Last active (UTC)",
        ],
        &cells,
        &TableLayout {
            right: &ITEM_NUMBER_COLUMNS,
            ..TableLayout::default()
        },
    )
}

/// The open tasks at `now_ms`, as a table, or a sentence when there are none.
pub fn open(open: &[OpenTask], now_ms: u64) -> String {
    if open.is_empty() {
        return "No open tasks".to_owned();
    }
    let cells: Vec<[String; 6]> = open
        .iter()
        .map(|task| {
            [
                task.task.clone(),
                task.agent.clone(),
                task.state.as_str().to_owned(),
                task.item_name.clone().unwrap_or_else(|| "-".to_owned()),
                age(now_ms.saturating_sub(task.since_ms)),
                task.question
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), question_start),
            ]
        })
        .collect();
    render_table(
        ["Task", "Agent", "State", "Item", "Age", "Question"],
        &cells,
        &TableLayout {
            right: &OPEN_NUMBER_COLUMNS,
            ..TableLayout::default()
        },
    )
}

/// What `link` did to `task`, with `name` the name of the item it is in now.
pub fn linked(task: &str, linked: &Linked, name: &str) -> String {
    match linked {
        Linked::Created { item } => format!("Task {task} is in a new item {item}: {name}"),
        Linked::Named { item } => {
            format!("Item {item} of task {task} is now also named by it: {name}")
        }
        Linked::Assigned { item } => format!("Task {task} is in item {item}: {name}"),
        Linked::Already { item } => format!("Task {task} is already in item {item}: {name}"),
        Linked::Merged {
            from,
            into,
            tasks,
            refs,
        } => {
            let refs: Vec<String> = refs.iter().map(Ref::label).collect();
            let refs = if refs.is_empty() {
                "no refs".to_owned()
            } else {
                format!("refs {}", refs.join(", "))
            };
            format!(
                "Merged item {from} into item {into} ({name}): moved tasks {} and {refs}",
                tasks.join(", ")
            )
        }
    }
}

/// What `work backfill` did, on one line.
pub fn backfilled(done: &Backfilled) -> String {
    format!(
        "Parsed {} outcomes without a guest report: {} named a branch or pull request, {} refs \
         were new, {} tasks were put in an item",
        done.outcomes, done.named, done.new_refs, done.assigned
    )
}

/// `ms` as the largest two units: `3d 4h`, `5h 12m`, `42m`, `30s`.
fn age(ms: u64) -> String {
    let seconds = ms / 1000;
    let (days, hours, minutes) = (seconds / 86_400, seconds / 3600 % 24, seconds / 60 % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m")
    } else {
        format!("{seconds}s")
    }
}

/// The start of a question's last line, where a guest asks it, with `…` when there is more.
fn question_start(question: &str) -> String {
    let last = question.trim_end().lines().last().unwrap_or_default();
    let start: String = last.chars().take(QUESTION_CHARS).collect();
    if start.len() < last.len() {
        format!("{start}…")
    } else {
        start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use turso::Connection;

    use crate::message::{Ended, Kind, Message, PullRequestRef, RefKind, TaskUsage, Work, WorkRef};
    use crate::report::usage;
    use crate::store::{Recorded, Ref, Store, items, read, record, ref_item};

    /// 2026-11-14 12:00 UTC.
    const NOON: u64 = 1_794_657_600_000;

    fn message(task: &str, kind: Kind, text: &str) -> Message {
        let (from, to) = if kind.sent_by_worker() {
            ("dev01", "host")
        } else {
            ("host", "dev01")
        };
        Message {
            task: task.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            kind,
            text: text.to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
            work: None,
            ended: None,
        }
    }

    /// A record of four tasks: `a1b2c3d4`, tagged with the issue, ended on the issue's branch
    /// with its pull request; `e5f6a7b8`, untagged, ended on the same branch; `c9d0e1f2`, tagged
    /// with the issue and still running; `f3a4b5c6`, untagged, asking a question on `main`.
    async fn recorded() -> (Store, Connection) {
        let store = Store::open_in_memory().await.unwrap();
        let mut conn = store.connect().unwrap();
        let ended = Some(Ended {
            repo: "bwks/sherpa".to_owned(),
            branch: Some("feat/580-work-items".to_owned()),
            pull_requests: vec![PullRequestRef {
                repo: "bwks/sherpa".to_owned(),
                number: 630,
                url: "https://github.com/bwks/sherpa/pull/630".to_owned(),
            }],
        });
        let tagged = |task: &str, text: &str| {
            let mut sent = message(task, Kind::Task, text);
            sent.model = Some("claude-opus-5-5".to_owned());
            sent.work = Some(Work {
                refs: vec![WorkRef {
                    repo: "bwks/sherpa".to_owned(),
                    kind: RefKind::Issue,
                    number: 580,
                }],
                session: None,
            });
            sent
        };
        let done = |task: &str, cost_usd: f64, ended: Option<Ended>| {
            let mut done = message(task, Kind::Completed, "done");
            done.model = Some("claude-opus-5-5".to_owned());
            done.task_usage = Some(TaskUsage {
                runs: 3,
                cost_usd,
                input: 120,
                output: 8_000,
                cache_creation: 30_000,
                cache_read: 400_000,
                ..TaskUsage::default()
            });
            done.ended = ended;
            done
        };
        let mut question = message(
            "f3a4b5c6",
            Kind::Question,
            "I read the logs.\n\nQUESTION: Restart the bus container now?",
        );
        question.ended = Some(Ended {
            repo: "bwks/sherpa".to_owned(),
            branch: None,
            pull_requests: Vec::new(),
        });
        let messages = [
            tagged("a1b2c3d4", "Implement PR 2 of the #580 design: work items"),
            done("a1b2c3d4", 4.2817, ended.clone()),
            message(
                "e5f6a7b8",
                Kind::Task,
                "Fix the failing clippy check on the PR",
            ),
            done("e5f6a7b8", 0.3561, ended),
            tagged("c9d0e1f2", "Update AGENT_BUS.md for work items"),
            message("c9d0e1f2", Kind::Progress, "Reading the docs"),
            message("f3a4b5c6", Kind::Task, "Why did the bus restart?"),
            question,
        ];
        for (index, sent) in (0_u64..).zip(&messages) {
            let recorded = Recorded {
                seq: index + 1,
                at_ms: NOON + index * 600_000,
                message: sent,
            };
            record(&mut conn, &recorded, Some("bwks/sherpa"))
                .await
                .unwrap();
        }
        (store, conn)
    }

    #[tokio::test]
    async fn work_show_usage_by_item_and_open_print_from_the_record() {
        let (_store, conn) = recorded().await;
        let issue = Ref::new("bwks/sherpa", RefKind::Issue, "580");
        let id = ref_item(&conn, &issue).await.unwrap().unwrap();
        let found = items::item(&conn, id).await.unwrap();

        let shown = item(&found);
        println!("$ agent-bus work show --issue 580\n{shown}\n");
        for expected in [
            "Item 1: #580",
            "bwks/sherpa branch feat/580-work-items",
            "bwks/sherpa PR #630",
            "a1b2c3d4",
            "e5f6a7b8",
            "c9d0e1f2",
            "$4.6378",
        ] {
            assert!(shown.contains(expected), "{expected} missing from\n{shown}");
        }
        assert!(!shown.contains("f3a4b5c6"), "{shown}");

        let rows = read::usage(&conn, "host", None, None, false, Some(id))
            .await
            .unwrap();
        let used = usage::table(&rows);
        println!("$ agent-bus usage --issue 580\nItem 1: #580\n{used}\n");
        assert_eq!(
            rows.iter().map(|row| row.task.as_str()).collect::<Vec<_>>(),
            ["a1b2c3d4", "e5f6a7b8"]
        );
        let total = used.lines().find(|line| line.contains("total")).unwrap();
        assert!(total.contains("$4.6378"), "{total}");

        let tasks = items::open_tasks(&conn).await.unwrap();
        let listed = open(&tasks, NOON + 8 * 3_600_000);
        println!("$ agent-bus open\n{listed}");
        assert_eq!(
            tasks
                .iter()
                .map(|task| (task.task.as_str(), task.item_name.as_deref()))
                .collect::<Vec<_>>(),
            [("c9d0e1f2", Some("#580")), ("f3a4b5c6", None)]
        );
        for expected in [
            "running",
            "question",
            "7h 20m",
            "QUESTION: Restart the bus container now?",
        ] {
            assert!(
                listed.contains(expected),
                "{expected} missing from\n{listed}"
            );
        }
        let json = serde_json::to_value(&tasks).unwrap();
        assert_eq!(
            json[1]["question"],
            "I read the logs.\n\nQUESTION: Restart the bus container now?"
        );
        assert_eq!(json[0]["item_name"], "#580");
    }

    #[tokio::test]
    async fn work_show_without_an_item_lists_every_item() {
        let (_store, conn) = recorded().await;
        let all = items::items(&conn).await.unwrap();
        let listed = items(&all);
        println!("$ agent-bus work show\n{listed}");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].tasks, 3);
        assert!(
            listed.contains("feat/580-work-items, #580, PR #630"),
            "{listed}"
        );
        // An item whose tasks have no cost yet shows $0.0000, not $-0.0000.
        record(
            &mut conn.clone(),
            &Recorded {
                seq: 100,
                at_ms: NOON,
                message: &{
                    let mut late = message("aaaa0000", Kind::Completed, "done");
                    late.ended = Some(Ended {
                        repo: "bwks/sherpa".to_owned(),
                        branch: Some("fix/free".to_owned()),
                        pull_requests: Vec::new(),
                    });
                    late
                },
            },
            None,
        )
        .await
        .unwrap();
        let listed = items(&items::items(&conn).await.unwrap());
        assert!(listed.contains("$0.0000"), "{listed}");
        assert!(!listed.contains("$-0"), "{listed}");
        assert_eq!(items(&[]), "No work items recorded");
    }

    #[test]
    fn an_age_shows_its_two_largest_units() {
        assert_eq!(age(30_000), "30s");
        assert_eq!(age(42 * 60_000 + 5_000), "42m");
        assert_eq!(age((5 * 60 + 12) * 60_000), "5h 12m");
        assert_eq!(age((3 * 24 + 4) * 3_600_000), "3d 4h");
    }

    #[test]
    fn a_question_shows_the_start_of_its_last_line() {
        assert_eq!(
            question_start("Generation section: no open pull request\n\nQUESTION: which one?"),
            "QUESTION: which one?"
        );
        let long = format!("QUESTION: {}", "x".repeat(100));
        assert!(question_start(&long).ends_with('…'));
    }
}
