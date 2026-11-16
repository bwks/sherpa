//! Which pull request's Generation section counts each task of a work item, and each response of
//! the host, without I/O. The store reads the facts (`store::generation::placements`); this
//! applies the rules of "Which pull request counts what" in `docs/architecture/AGENT_BUS.md`.
//!
//! - A task's **home** is one of the pull requests it named in its own item: those of the host's
//!   repository first, then by source (`manual`, `guest`, `parsed`, `brief`), then the one named
//!   last, then the lowest. Its other pull requests show it as counted on its home.
//! - A task that named none is **brought forward** to the home of the item's next task, in the
//!   record's order, that has one; until there is one it is in no section.
//! - A response of the host goes to the pull request its focus named, else to where the item's
//!   task with the latest event at or before it is counted, else (before any) to where the
//!   first task is; a response after the final write of a merged or closed pull request goes
//!   on to the item's next pull request.

use std::collections::HashMap;

use serde::Serialize;

/// The sources of a task's refs, most trusted first.
const SOURCES: [&str; 4] = ["manual", "guest", "parsed", "brief"];

/// A pull request, by repository and number.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct PullRequestId {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
}

/// A pull request a task named in its own item: how, and in which message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    pub pull_request: PullRequestId,
    /// `brief`, `guest`, `parsed` or `manual`.
    pub source: String,
    /// The message that named it first.
    pub seq: u64,
}

/// Where a task is counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Placement {
    /// The pull request whose section counts it; `None` while there is none.
    pub pull_request: Option<PullRequestId>,
    /// Whether it named no pull request of its own and is counted on a later task's.
    pub brought_forward: bool,
    /// Its other pull requests, whose sections point to `pull_request`.
    pub elsewhere: Vec<PullRequestId>,
}

/// One item's tasks in the record's order, where each is counted, and the times of their events
/// (`task`, `answer`, `question` and outcome messages), oldest first, with the task's index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemPlan {
    pub tasks: Vec<String>,
    pub placements: Vec<Placement>,
    pub events: Vec<(u64, usize)>,
}

/// Every item's plan, and when each merged or closed pull request had its final write.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placements {
    pub items: HashMap<i64, ItemPlan>,
    pub frozen: HashMap<PullRequestId, u64>,
}

impl Placements {
    /// Where `task` is counted, with its item; `None` for a task in no item.
    pub fn task(&self, task: &str) -> Option<(i64, &Placement)> {
        self.items.iter().find_map(|(item, plan)| {
            plan.tasks
                .iter()
                .position(|known| known == task)
                .and_then(|index| plan.placements.get(index))
                .map(|placement| (*item, placement))
        })
    }

    /// The pull request that counts a response of the host made at `at_ms` and charged to
    /// `item`, with `explicit` the pull request its focus named or it was pinned to. `None`:
    /// in no pull request (yet).
    pub fn host(
        &self,
        item: i64,
        explicit: Option<&PullRequestId>,
        at_ms: u64,
    ) -> Option<PullRequestId> {
        let plan = self.items.get(&item);
        let mut target = match explicit {
            Some(pull_request) => Some(pull_request.clone()),
            None => plan.and_then(|plan| {
                let index = plan
                    .events
                    .iter()
                    .rev()
                    .find(|(event_ms, _)| *event_ms <= at_ms)
                    .map_or(0, |(_, index)| *index);
                plan.placements
                    .get(index)
                    .and_then(|placement| placement.pull_request.clone())
            }),
        };
        let order = plan.map_or_else(Vec::new, order);
        while let Some(pull_request) = &target
            && self
                .frozen
                .get(pull_request)
                .is_some_and(|final_ms| at_ms > *final_ms)
        {
            target = order
                .iter()
                .position(|known| known == pull_request)
                .and_then(|index| order.get(index + 1))
                .cloned();
        }
        target
    }
}

/// Where each of `tasks` (an item's, in the record's order, each with the pull requests it
/// named in the item) is counted, with `home_repo` the host's repository.
pub fn place(tasks: &[Vec<Named>], home_repo: Option<&str>) -> Vec<Placement> {
    let homes: Vec<Option<&PullRequestId>> =
        tasks.iter().map(|named| home(named, home_repo)).collect();
    homes
        .iter()
        .enumerate()
        .map(|(index, own)| {
            let Some(own) = own else {
                let next = homes[index + 1..].iter().find_map(|later| *later);
                return Placement {
                    pull_request: next.cloned(),
                    brought_forward: next.is_some(),
                    elsewhere: Vec::new(),
                };
            };
            let mut elsewhere: Vec<PullRequestId> = tasks[index]
                .iter()
                .map(|named| named.pull_request.clone())
                .filter(|pull_request| pull_request != *own)
                .collect();
            elsewhere.sort();
            elsewhere.dedup();
            Placement {
                pull_request: Some((*own).clone()),
                brought_forward: false,
                elsewhere,
            }
        })
        .collect()
}

/// The home of a task that named `named`: one in `home_repo` when it has any, then the most
/// trusted source, the one named last, the lowest.
fn home<'a>(named: &'a [Named], home_repo: Option<&str>) -> Option<&'a PullRequestId> {
    let in_home = named
        .iter()
        .any(|one| Some(one.pull_request.repo.as_str()) == home_repo);
    named
        .iter()
        .filter(|one| !in_home || Some(one.pull_request.repo.as_str()) == home_repo)
        .min_by(|a, b| {
            trust(&a.source)
                .cmp(&trust(&b.source))
                .then(b.seq.cmp(&a.seq))
                .then(a.pull_request.cmp(&b.pull_request))
        })
        .map(|one| &one.pull_request)
}

/// How trusted `source` is: lower is more.
fn trust(source: &str) -> usize {
    SOURCES
        .iter()
        .position(|known| *known == source)
        .unwrap_or(SOURCES.len())
}

/// The pull requests that count `plan`'s tasks, in the order of their first task.
fn order(plan: &ItemPlan) -> Vec<PullRequestId> {
    let mut found: Vec<PullRequestId> = Vec::new();
    for placement in &plan.placements {
        if let Some(pull_request) = &placement.pull_request
            && !found.contains(pull_request)
        {
            found.push(pull_request.clone());
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHERPA: &str = "bwks/sherpa";
    const DOCS: &str = "bwks/sherpa-docs";

    fn pr(repo: &str, number: u64) -> PullRequestId {
        PullRequestId {
            repo: repo.to_owned(),
            number,
        }
    }

    fn named(repo: &str, number: u64, source: &str, seq: u64) -> Named {
        Named {
            pull_request: pr(repo, number),
            source: source.to_owned(),
            seq,
        }
    }

    fn own(pull_request: PullRequestId, elsewhere: Vec<PullRequestId>) -> Placement {
        Placement {
            pull_request: Some(pull_request),
            brought_forward: false,
            elsewhere,
        }
    }

    fn forward(pull_request: PullRequestId) -> Placement {
        Placement {
            pull_request: Some(pull_request),
            brought_forward: true,
            elsewhere: Vec::new(),
        }
    }

    #[test]
    fn a_task_with_one_pull_request_is_counted_there() {
        assert_eq!(
            place(&[vec![named(SHERPA, 665, "guest", 10)]], Some(SHERPA)),
            [own(pr(SHERPA, 665), vec![])]
        );
    }

    #[test]
    fn the_host_repositorys_pull_request_is_home_and_the_docs_one_points_to_it() {
        // 1d6ad1ad of #648: one outcome named both.
        let tasks = [vec![
            named(DOCS, 142, "guest", 20),
            named(SHERPA, 666, "guest", 20),
        ]];
        assert_eq!(
            place(&tasks, Some(SHERPA)),
            [own(pr(SHERPA, 666), vec![pr(DOCS, 142)])]
        );
        // Without the host's repository, the lowest repository and number break the tie.
        assert_eq!(
            place(&tasks, None),
            [own(pr(SHERPA, 666), vec![pr(DOCS, 142)])]
        );
        // A task whose only pull requests are in another repository is counted there.
        assert_eq!(
            place(&[vec![named(DOCS, 141, "guest", 5)]], Some(SHERPA)),
            [own(pr(DOCS, 141), vec![])]
        );
    }

    #[test]
    fn two_pull_requests_in_one_repository_go_by_source_then_the_last_named_then_the_lowest() {
        // A brief tag and where the run ended: the guest's report wins.
        assert_eq!(
            place(
                &[vec![
                    named(SHERPA, 7, "brief", 1),
                    named(SHERPA, 8, "guest", 2)
                ]],
                Some(SHERPA)
            ),
            [own(pr(SHERPA, 8), vec![pr(SHERPA, 7)])]
        );
        // `link` wins over the guest.
        assert_eq!(
            place(
                &[vec![
                    named(SHERPA, 7, "manual", 9),
                    named(SHERPA, 8, "guest", 2)
                ]],
                Some(SHERPA)
            )[0]
            .pull_request,
            Some(pr(SHERPA, 7))
        );
        // Same source: the one named last, then the lowest.
        assert_eq!(
            place(
                &[vec![
                    named(SHERPA, 7, "guest", 2),
                    named(SHERPA, 8, "guest", 5)
                ]],
                Some(SHERPA)
            )[0]
            .pull_request,
            Some(pr(SHERPA, 8))
        );
        assert_eq!(
            place(
                &[vec![
                    named(SHERPA, 9, "guest", 5),
                    named(SHERPA, 8, "guest", 5)
                ]],
                Some(SHERPA)
            )[0]
            .pull_request,
            Some(pr(SHERPA, 8))
        );
    }

    #[test]
    fn tasks_without_a_pull_request_are_brought_forward_to_the_next_tasks() {
        // #648: two design tasks, then PR 0, PR 1 with its docs PR, and PR 2 with its own.
        let tasks = [
            vec![],
            vec![],
            vec![named(SHERPA, 665, "guest", 30)],
            vec![
                named(SHERPA, 666, "guest", 40),
                named(DOCS, 142, "guest", 40),
            ],
            vec![
                named(SHERPA, 667, "guest", 50),
                named(DOCS, 143, "guest", 50),
            ],
        ];
        assert_eq!(
            place(&tasks, Some(SHERPA)),
            [
                forward(pr(SHERPA, 665)),
                forward(pr(SHERPA, 665)),
                own(pr(SHERPA, 665), vec![]),
                own(pr(SHERPA, 666), vec![pr(DOCS, 142)]),
                own(pr(SHERPA, 667), vec![pr(DOCS, 143)]),
            ]
        );
        // A brief tag alone is a home: a review task that ended on main.
        assert_eq!(
            place(
                &[vec![], vec![named(SHERPA, 668, "brief", 3)]],
                Some(SHERPA)
            ),
            [forward(pr(SHERPA, 668)), own(pr(SHERPA, 668), vec![])]
        );
        // Nothing later to bring forward to: in no section yet.
        assert_eq!(
            place(&[vec![named(SHERPA, 1, "guest", 1)], vec![]], Some(SHERPA))[1],
            Placement::default()
        );
    }

    /// #648's plan: design tasks at 0 and 10, PR 0 (#665) at 20-30, PR 1 (#666) at 40-50, PR 2
    /// (#667) at 60-70.
    fn plan_648() -> Placements {
        let tasks = [
            vec![],
            vec![],
            vec![named(SHERPA, 665, "guest", 30)],
            vec![named(SHERPA, 666, "guest", 50)],
            vec![named(SHERPA, 667, "guest", 70)],
        ];
        let plan = ItemPlan {
            tasks: ["d1", "d2", "p0", "p1", "p2"].map(str::to_owned).to_vec(),
            placements: place(&tasks, Some(SHERPA)),
            events: vec![
                (0, 0),
                (5, 0),
                (10, 1),
                (15, 1),
                (20, 2),
                (30, 2),
                (40, 3),
                (50, 3),
                (60, 4),
                (70, 4),
            ],
        };
        Placements {
            items: HashMap::from([(1, plan)]),
            frozen: HashMap::new(),
        }
    }

    #[test]
    fn host_work_follows_the_last_task_event_unless_its_focus_names_a_pull_request() {
        let placements = plan_648();
        // Writing the first brief, before any task: where the first task goes.
        assert_eq!(placements.host(1, None, 0), Some(pr(SHERPA, 665)));
        // Reading a design result: brought forward with it.
        assert_eq!(placements.host(1, None, 12), Some(pr(SHERPA, 665)));
        assert_eq!(placements.host(1, None, 45), Some(pr(SHERPA, 666)));
        assert_eq!(placements.host(1, None, 99), Some(pr(SHERPA, 667)));
        // `work use --pr 665` or `send --pr 665` wins.
        assert_eq!(
            placements.host(1, Some(&pr(SHERPA, 665)), 99),
            Some(pr(SHERPA, 665))
        );
        // Another item, or an item with no task: nothing.
        assert_eq!(placements.host(2, None, 5), None);
    }

    #[test]
    fn host_work_after_a_final_write_goes_on_to_the_items_next_pull_request() {
        let mut placements = plan_648();
        // #665 had its final write at 32, #666 at 55.
        placements.frozen.insert(pr(SHERPA, 665), 32);
        placements.frozen.insert(pr(SHERPA, 666), 55);
        assert_eq!(placements.host(1, None, 31), Some(pr(SHERPA, 665)));
        // The last event is #665's task, but its section is frozen: on to #666.
        assert_eq!(placements.host(1, None, 35), Some(pr(SHERPA, 666)));
        // Past both: on to #667, even when the focus named #665.
        assert_eq!(placements.host(1, None, 56), Some(pr(SHERPA, 667)));
        assert_eq!(
            placements.host(1, Some(&pr(SHERPA, 665)), 56),
            Some(pr(SHERPA, 667))
        );
        // After the item's last pull request is frozen: in none.
        placements.frozen.insert(pr(SHERPA, 667), 80);
        assert_eq!(placements.host(1, None, 81), None);
    }

    #[test]
    fn a_task_is_found_with_its_item() {
        let placements = plan_648();
        let (item, placement) = placements.task("d2").unwrap();
        assert_eq!((item, placement), (1, &forward(pr(SHERPA, 665))));
        assert!(placements.task("other").is_none());
    }
}
