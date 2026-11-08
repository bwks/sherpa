//! What `agent-bus agents` shows about each agent, and the heartbeat it is built from.
//!
//! A guest agent keeps its status in the `JetStream` key-value bucket `AGENT_STATUS`, under its
//! own name: whether it is running, the task it works on and how many tasks wait in its local
//! queue. It writes the status on every change and every `HEARTBEAT`, and once more, marked
//! offline, when it stops. The host reads the bucket and each inbox consumer's info, and turns
//! them into one row per agent here. See `docs/architecture/AGENT_BUS.md` for why the heartbeat,
//! and not the consumer's pull requests, says whether a guest is online.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::agents::Role;
use crate::files::millis;

/// The key-value bucket of guest statuses; its stream is `KV_AGENT_STATUS`.
pub const BUCKET: &str = "AGENT_STATUS";
/// How often a guest agent writes its status when nothing changes.
pub const HEARTBEAT: Duration = Duration::from_secs(10);
/// A status older than this is stale: three heartbeats missed, and some slack.
pub const STALE: Duration = Duration::from_secs(35);

/// The status a guest agent writes, as JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// False only in the last status, written as the agent stops.
    pub online: bool,
    /// When the guest wrote it, by the guest's clock, in milliseconds since the Unix epoch.
    pub at_ms: u64,
    /// The task Claude Code works on.
    pub running: Option<Running>,
    /// Tasks and answers read from the inbox, waiting in the guest's local queue.
    pub queued: u64,
}

/// The running task, and when it started by the guest's clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Running {
    pub task: String,
    pub since_ms: u64,
}

/// A status as the bus stored it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heartbeat {
    pub status: Status,
    /// When the bus stored it, by the bus's clock, in milliseconds since the Unix epoch. The bus
    /// runs on the host, so this and the host's clock agree; the guest's may not.
    pub stored_ms: u64,
}

/// What the bus says about an agent's inbox consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InboxState {
    /// Messages not yet read, or read and not acknowledged.
    pub unread: u64,
    /// Whether a reader holds a pull request on the inbox now.
    pub reader_attached: bool,
    /// When the bus last delivered a message from it, in milliseconds since the Unix epoch.
    pub last_delivered_ms: Option<u64>,
}

/// Everything known about one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    pub name: String,
    pub role: Role,
    /// `None` when the agent has no inbox on the bus.
    pub inbox: Option<InboxState>,
    /// `None` when the agent never wrote a status, or the bus has no status bucket.
    pub heartbeat: Option<Heartbeat>,
}

/// Whether an agent is there now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Online {
    Yes,
    No,
    /// No status, but a reader holds the inbox: a guest agent from before the heartbeat, or
    /// some other reader. It may or may not be working.
    Unreported,
}

/// The running task in a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunningRow {
    pub task: String,
    /// How long it has run, in seconds.
    pub age_secs: u64,
}

/// One agent in `agent-bus agents`, also its JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    pub name: String,
    pub role: Role,
    pub online: Online,
    /// Only while online.
    pub running: Option<RunningRow>,
    /// Tasks and answers waiting for a guest: `inbox_unread` and `local_queue` together. `None`
    /// for the host: what waits in its inbox is progress and outcomes for a listener, not tasks,
    /// and is only in `inbox_unread`.
    pub queued: Option<u64>,
    /// Messages on the bus the agent has not read; `None` when it has no inbox.
    pub inbox_unread: Option<u64>,
    /// The guest's local queue at its last status; `None` without a status.
    pub local_queue: Option<u64>,
    pub reader_attached: bool,
    /// Milliseconds since the Unix epoch; `None` when never seen.
    pub last_active_ms: Option<u64>,
    pub last_active_age_secs: Option<u64>,
}

impl Online {
    /// The word in the table.
    pub fn word(self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::Unreported => "unreported",
        }
    }
}

/// The row for `observed`, `now_ms` being the host's clock and `stale` the age at which a status
/// no longer counts (`STALE`).
///
/// A guest is online while its last status says so and is fresh. Its running task is shown only
/// then: a stale status's task is not running now (a guest that restarts reports it failed).
/// Its last activity is its last status. The host has no status: it is online while a listener
/// holds its inbox, and was last active when a message was last delivered to it.
pub fn row(observed: &Observed, now_ms: u64, stale: Duration) -> Row {
    let inbox = observed.inbox;
    let reader_attached = inbox.is_some_and(|inbox| inbox.reader_attached);
    let inbox_unread = inbox.map(|inbox| inbox.unread);
    let heartbeat = match observed.role {
        Role::Host => None,
        Role::Guest => observed.heartbeat.as_ref(),
    };
    let (online, running, local_queue, last_active_ms) = if let Some(heartbeat) = heartbeat {
        let fresh = now_ms.saturating_sub(heartbeat.stored_ms) <= millis(stale);
        let online = if heartbeat.status.online && fresh {
            Online::Yes
        } else {
            Online::No
        };
        let running = if online == Online::Yes {
            heartbeat
                .status
                .running
                .as_ref()
                .map(|running| running_row(running, heartbeat, now_ms))
        } else {
            None
        };
        (
            online,
            running,
            Some(heartbeat.status.queued),
            Some(heartbeat.stored_ms),
        )
    } else {
        let online = match (observed.role, reader_attached) {
            (Role::Host, true) => Online::Yes,
            (Role::Guest, true) => Online::Unreported,
            (_, false) => Online::No,
        };
        let last_delivered = inbox.and_then(|inbox| inbox.last_delivered_ms);
        (online, None, None, last_delivered)
    };
    Row {
        name: observed.name.clone(),
        role: observed.role,
        online,
        running,
        queued: match observed.role {
            Role::Host => None,
            Role::Guest => Some(
                inbox_unread
                    .unwrap_or(0)
                    .saturating_add(local_queue.unwrap_or(0)),
            ),
        },
        inbox_unread,
        local_queue,
        reader_attached,
        last_active_ms,
        last_active_age_secs: last_active_ms.map(|at| now_ms.saturating_sub(at) / 1000),
    }
}

/// How long `running` has run: by the guest's clock up to its status, then by the host's since
/// the bus stored it, so a guest clock set wrong does not skew it.
fn running_row(running: &Running, heartbeat: &Heartbeat, now_ms: u64) -> RunningRow {
    let before = heartbeat.status.at_ms.saturating_sub(running.since_ms);
    let since = now_ms.saturating_sub(heartbeat.stored_ms);
    RunningRow {
        task: running.task.clone(),
        age_secs: before.saturating_add(since) / 1000,
    }
}

/// `secs` as its two largest units: `45s`, `3m 05s`, `2h 07m`, `4d 02h`.
pub fn age(secs: u64) -> String {
    let (days, hours, minutes, seconds) =
        (secs / 86_400, secs / 3600 % 24, secs / 60 % 60, secs % 60);
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000_000_000;

    fn guest(inbox: Option<InboxState>, heartbeat: Option<Heartbeat>) -> Observed {
        Observed {
            name: "dev".to_owned(),
            role: Role::Guest,
            inbox,
            heartbeat,
        }
    }

    fn inbox(unread: u64, reader_attached: bool) -> InboxState {
        InboxState {
            unread,
            reader_attached,
            last_delivered_ms: Some(NOW - 600_000),
        }
    }

    fn beat(online: bool, stored_ago_ms: u64, running: Option<Running>, queued: u64) -> Heartbeat {
        Heartbeat {
            status: Status {
                online,
                at_ms: NOW - stored_ago_ms,
                running,
                queued,
            },
            stored_ms: NOW - stored_ago_ms,
        }
    }

    #[test]
    fn a_fresh_status_is_online_with_its_task_and_queue() {
        let running = Running {
            task: "t1".to_owned(),
            since_ms: NOW - 125_000,
        };
        let row = row(
            &guest(
                Some(inbox(2, true)),
                Some(beat(true, 4_000, Some(running), 3)),
            ),
            NOW,
            STALE,
        );
        assert_eq!(row.online, Online::Yes);
        assert_eq!(
            row.running,
            Some(RunningRow {
                task: "t1".to_owned(),
                age_secs: 125
            })
        );
        assert_eq!(
            (row.queued, row.inbox_unread, row.local_queue),
            (Some(5), Some(2), Some(3))
        );
        assert_eq!(row.last_active_ms, Some(NOW - 4_000));
        assert_eq!(row.last_active_age_secs, Some(4));
    }

    #[test]
    fn a_stale_status_is_offline_and_its_task_is_not_shown() {
        let running = Running {
            task: "t1".to_owned(),
            since_ms: NOW - 900_000,
        };
        let stale = millis(STALE) + 1;
        // A reader still attached does not make a stale guest online: it may be hung.
        let row = row(
            &guest(
                Some(inbox(0, true)),
                Some(beat(true, stale, Some(running), 1)),
            ),
            NOW,
            STALE,
        );
        assert_eq!(row.online, Online::No);
        assert_eq!(row.running, None);
        assert_eq!(row.local_queue, Some(1));
        assert_eq!(row.last_active_ms, Some(NOW - stale));
        // At the limit it is still fresh.
        let at_limit = super::row(
            &guest(None, Some(beat(true, millis(STALE), None, 0))),
            NOW,
            STALE,
        );
        assert_eq!(at_limit.online, Online::Yes);
        // The threshold is the caller's: a status 3 s old is stale against 2 s.
        let short = super::row(
            &guest(None, Some(beat(true, 3_000, None, 0))),
            NOW,
            Duration::from_secs(2),
        );
        assert_eq!(short.online, Online::No);
    }

    #[test]
    fn a_guest_that_said_it_stopped_is_offline_at_once() {
        let row = row(
            &guest(Some(inbox(4, false)), Some(beat(false, 1_000, None, 0))),
            NOW,
            STALE,
        );
        assert_eq!(row.online, Online::No);
        assert_eq!(row.queued, Some(4));
    }

    #[test]
    fn without_a_status_a_reader_is_unreported_and_none_is_offline() {
        let reader = row(&guest(Some(inbox(0, true)), None), NOW, STALE);
        assert_eq!(reader.online, Online::Unreported);
        assert_eq!(reader.local_queue, None);
        // The last delivery is the only activity known.
        assert_eq!(reader.last_active_ms, Some(NOW - 600_000));
        let nobody = row(&guest(None, None), NOW, STALE);
        assert_eq!(nobody.online, Online::No);
        assert_eq!(
            (nobody.queued, nobody.inbox_unread, nobody.last_active_ms),
            (Some(0), None, None)
        );
    }

    #[test]
    fn the_host_is_online_while_a_listener_holds_its_inbox() {
        let host = |inbox| Observed {
            name: "host".to_owned(),
            role: Role::Host,
            inbox: Some(inbox),
            // A status under the host's name is not the host's: only guests write them.
            heartbeat: Some(beat(true, 0, None, 9)),
        };
        let listening = row(&host(inbox(1, true)), NOW, STALE);
        assert_eq!(listening.online, Online::Yes);
        assert_eq!(listening.local_queue, None);
        assert_eq!(listening.last_active_age_secs, Some(600));
        assert_eq!(row(&host(inbox(0, false)), NOW, STALE).online, Online::No);
    }

    #[test]
    fn what_waits_in_the_host_inbox_is_unread_not_queued() {
        // Progress and outcomes from guests, waiting for a listener: not tasks for the host.
        let host = Observed {
            name: "host".to_owned(),
            role: Role::Host,
            inbox: Some(inbox(2, false)),
            heartbeat: None,
        };
        let row = row(&host, NOW, STALE);
        assert_eq!((row.queued, row.inbox_unread), (None, Some(2)));
        let json = serde_json::to_value(&row).unwrap();
        assert!(json["queued"].is_null(), "{json}");
        assert_eq!(json["inbox_unread"], 2);
    }

    #[test]
    fn a_running_age_is_not_skewed_by_the_guest_clock() {
        // The guest's clock is an hour behind: its timestamps are an hour early, but the task
        // has run 60s by it, and the status was stored 5s ago by the bus.
        let skew = 3_600_000;
        let heartbeat = Heartbeat {
            status: Status {
                online: true,
                at_ms: NOW - 5_000 - skew,
                running: Some(Running {
                    task: "t".to_owned(),
                    since_ms: NOW - 65_000 - skew,
                }),
                queued: 0,
            },
            stored_ms: NOW - 5_000,
        };
        let row = row(&guest(None, Some(heartbeat)), NOW, STALE);
        assert_eq!(row.running.map(|running| running.age_secs), Some(65));
    }

    #[test]
    fn the_json_names_every_field() {
        let row = row(&guest(Some(inbox(0, true)), None), NOW, STALE);
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json["online"], "unreported");
        assert_eq!(json["role"], "guest");
        assert!(json["running"].is_null());
        for field in [
            "name",
            "queued",
            "inbox_unread",
            "local_queue",
            "reader_attached",
            "last_active_ms",
            "last_active_age_secs",
        ] {
            assert!(json.get(field).is_some(), "{field} missing from {json}");
        }
    }

    #[test]
    fn a_status_round_trips() {
        let status = Status {
            online: true,
            at_ms: 7,
            running: Some(Running {
                task: "t".to_owned(),
                since_ms: 3,
            }),
            queued: 2,
        };
        let text = serde_json::to_string(&status).unwrap();
        assert_eq!(serde_json::from_str::<Status>(&text).unwrap(), status);
    }

    #[test]
    fn ages_show_their_two_largest_units() {
        assert_eq!(age(0), "0s");
        assert_eq!(age(45), "45s");
        assert_eq!(age(185), "3m 05s");
        assert_eq!(age(2 * 3600 + 7 * 60 + 9), "2h 07m");
        assert_eq!(age(4 * 86_400 + 2 * 3600), "4d 02h");
    }
}
