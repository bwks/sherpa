//! Opening the host's record for one command: the only way any process reads or writes it.
//!
//! A command that needs the record (`usage`, `show`, `history`) connects to the bus, takes
//! `record.lock` in the state directory, opens `agent-bus.db`, catches it up from the durable
//! consumer `_record` (each message recorded in one transaction, committed, then
//! acknowledged; its stream sequence is its key, so one delivered again changes nothing), runs
//! its query, and closes the record and the lock when it drops `Record`. There is no
//! long-running writer: between commands the stream keeps what the record has not taken yet.
//!
//! Catching up reads until the consumer has nothing left to deliver, however many pulls that
//! takes. When the bus stops delivering while messages still wait, the command says it did not
//! catch up. Messages that are not taken (a forged sender, not an agent message) are
//! acknowledged and counted, and the command prints one line with the counts.
//!
//! Turso holds its file for one process, and its lock says nothing about who holds it, so
//! `record.lock` is the arbiter. Its holder writes its PID into it; a command that cannot take
//! it within `LOCK_WAIT` fails naming that PID. When the bus cannot be reached, or the host's
//! seed is missing, the command answers from what is recorded and says it did not catch up.

use std::io::{Stderr, stderr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Error, Result, bail};
use tokio::time::{Instant, sleep, timeout};
use tracing::{debug, instrument};
use turso::Connection;

use crate::channel::{Channel, Delivery, DropReason, Endpoint, Inbox, Position, Received};
use crate::files::{FileLock, lock_holder, try_lock_exclusive};
use crate::store::{self, Recorded, Store, record};

/// The lock whose holder alone opens the record.
const LOCK_FILE: &str = "record.lock";
/// How long a command waits for another to let go of the record. A command holds it for as
/// long as it takes to catch up and query: milliseconds, or a few milliseconds a message for a
/// backlog (about half a minute for 6,000, almost all of it one commit per message).
const LOCK_WAIT: Duration = Duration::from_secs(5);
/// How often a waiting command tries the lock again.
const LOCK_RETRY: Duration = Duration::from_millis(50);
/// How long catching up waits for the bus to deliver the next message while messages still
/// wait. Past it, the bus has stopped delivering, and the record is not caught up.
const CATCH_UP_IDLE: Duration = Duration::from_secs(2);
/// From how many waiting messages catching up says what it is doing.
const CATCH_UP_NOTICE: u64 = 100;

/// Where the record and its lock are.
#[derive(Debug, Clone)]
pub struct Paths {
    pub record: PathBuf,
    pub lock: PathBuf,
}

/// What catching up from the bus needs: where it is, and the host's seed.
pub struct BusAccess {
    pub server: Endpoint,
    pub seed: String,
}

/// What catching up read from the bus.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    /// Messages recorded.
    pub recorded: u64,
    /// Messages acknowledged without recording: their sender is not their subject's.
    pub foreign_sender: u64,
    /// Messages acknowledged without recording: they are not agent messages.
    pub not_messages: u64,
}

/// How catching up went.
#[derive(Debug)]
pub enum CatchUp {
    /// The record took everything waiting on the bus, and is current.
    Read(Tally),
    /// The record could not be caught up; it holds what was recorded before.
    Failed(Error),
}

/// The record, open and caught up for one command. Dropping it closes the record, then
/// releases the lock.
pub struct Record {
    // Fields drop in order: the connection and the database close before the lock is released.
    conn: Connection,
    _store: Store,
    _lock: FileLock,
    catch_up: CatchUp,
}

impl Paths {
    /// The record's files in the state directory `dir`.
    pub fn in_dir(dir: &Path) -> Self {
        Self {
            record: dir.join(store::FILE),
            lock: dir.join(LOCK_FILE),
        }
    }
}

impl Record {
    /// Open the record at `paths` for one command, caught up from the bus through `access`.
    /// When `access` is an error, the bus cannot be reached, or a message cannot be recorded,
    /// the record is opened as it is and `catch_up` says why.
    ///
    /// # Errors
    ///
    /// Fails when another process holds the record for longer than `LOCK_WAIT`, or the record
    /// cannot be opened.
    #[instrument(skip_all, fields(record = %paths.record.display()))]
    pub async fn open(paths: &Paths, access: Result<BusAccess>) -> Result<Self> {
        // Connecting can take a while when the bus is down, so it happens before the lock.
        let channel = match access {
            Ok(access) => Channel::connect(&access.server, &access.seed).await,
            Err(error) => Err(error),
        };
        let lock = lock_within(&paths.lock, LOCK_WAIT).await?;
        let store = Store::open(&paths.record).await?;
        let mut conn = store.connect()?;
        let catch_up = match channel {
            Ok(channel) => match catch_up(&mut conn, &channel).await {
                Ok(read) => CatchUp::Read(read),
                Err(error) => CatchUp::Failed(error),
            },
            Err(error) => CatchUp::Failed(error),
        };
        if let CatchUp::Read(tally) = &catch_up {
            debug!(?tally, "caught the record up from the bus");
        }
        Ok(Self {
            conn,
            _store: store,
            _lock: lock,
            catch_up,
        })
    }

    /// The connection to query the record on.
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// How catching up went.
    #[cfg(test)]
    pub fn catch_up(&self) -> &CatchUp {
        &self.catch_up
    }

    /// The warning to print when the record was not caught up; `None` when it is current.
    pub fn behind(&self) -> Option<String> {
        match &self.catch_up {
            CatchUp::Read(_) => None,
            CatchUp::Failed(error) => Some(format!(
                "Showing what was recorded before: not caught up with the bus: {error:#}"
            )),
        }
    }
}

/// Take the lock at `path`, waiting up to `wait` for another holder to let it go.
///
/// # Errors
///
/// Fails when the lock cannot be taken, or is still held after `wait`; the error names the
/// holder's PID when it is known.
#[instrument(level = "debug", skip_all, fields(path = %path.display()))]
async fn lock_within(path: &Path, wait: Duration) -> Result<FileLock> {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(lock) = try_lock_exclusive(path)? {
            return Ok(lock);
        }
        if Instant::now() >= deadline {
            let holder = lock_holder(path).map_or_else(
                || "another process".to_owned(),
                |pid| format!("process {pid}"),
            );
            bail!(
                "The record is busy: {holder} has held {} for more than {}s. A command holds \
                 it only while it catches up and reads; if that process is stuck, stop it",
                path.display(),
                wait.as_secs_f32()
            );
        }
        sleep(LOCK_RETRY).await;
    }
}

/// Record what waits on the `_record` consumer, until it has nothing left to deliver, and
/// return what was read. Prints one line counting the messages acknowledged without recording,
/// when there are any, whether or not catching up finished.
///
/// # Errors
///
/// Fails when the consumer cannot be read, the bus stops delivering while messages still wait,
/// or a message cannot be recorded; that message is not acknowledged, so a later command records
/// it.
#[instrument(level = "debug", skip_all)]
async fn catch_up(conn: &mut Connection, channel: &Channel) -> Result<Tally> {
    let (mut inbox, waiting) = channel.recorder(stderr()).await?;
    let mut tally = Tally::default();
    if waiting == 0 {
        return Ok(tally);
    }
    if waiting >= CATCH_UP_NOTICE {
        eprintln!("Recording {waiting} messages from the bus");
    }
    let read = read_until_current(conn, &mut inbox, waiting, &mut tally).await;
    if let Some(summary) = dropped_summary(&tally) {
        eprintln!("{summary}");
    }
    read.map(|()| tally)
}

/// Record or drop each message `inbox` delivers, counting them in `tally`, until a message
/// says the consumer has nothing left to deliver. `waiting` is how many waited when reading
/// started. Each read is timed on its own, so a long backlog takes as long as it takes; only a
/// bus that delivers nothing for `CATCH_UP_IDLE` while messages still wait ends it early.
///
/// # Errors
///
/// Fails when the bus stops delivering while messages still wait, the consumer closes or cannot
/// be read, or a message cannot be recorded.
#[instrument(level = "debug", skip_all, fields(waiting = waiting))]
async fn read_until_current(
    conn: &mut Connection,
    inbox: &mut Inbox<Stderr>,
    waiting: u64,
    tally: &mut Tally,
) -> Result<()> {
    let mut pending = waiting;
    loop {
        let Ok(next) = timeout(CATCH_UP_IDLE, inbox.receive()).await else {
            bail!(
                "the bus delivered nothing for {}s with {pending} messages still waiting, \
                 after {} were recorded",
                CATCH_UP_IDLE.as_secs_f32(),
                tally.recorded
            );
        };
        let Some(received) = next? else {
            bail!(
                "the recorder's consumer closed with {pending} messages still waiting, after {} \
                 were recorded",
                tally.recorded
            );
        };
        let position = match received {
            Received::Taken(delivery) => {
                let position = record_delivery(conn, &delivery).await?;
                tally.recorded += 1;
                position
            }
            Received::Dropped(dropped) => {
                match dropped.reason {
                    DropReason::ForeignSender => tally.foreign_sender += 1,
                    DropReason::NotAMessage => tally.not_messages += 1,
                }
                dropped.position
            }
        };
        pending = position.pending;
        if pending == 0 {
            return Ok(());
        }
    }
}

/// The one line that counts the messages `tally` acknowledged without recording, by reason;
/// `None` when there were none.
fn dropped_summary(tally: &Tally) -> Option<String> {
    let mut reasons = Vec::new();
    if tally.foreign_sender > 0 {
        reasons.push(format!(
            "{} whose sender is not the subject's",
            tally.foreign_sender
        ));
    }
    if tally.not_messages > 0 {
        reasons.push(format!(
            "{} that are not agent messages",
            tally.not_messages
        ));
    }
    if reasons.is_empty() {
        return None;
    }
    Some(format!(
        "Acknowledged without recording: {}",
        reasons.join(", ")
    ))
}

/// Record `delivery`, commit, then acknowledge it, and return its position. An acknowledgement
/// the bus does not take is reported only: the message comes again and is skipped.
///
/// # Errors
///
/// Fails when the message has no position or cannot be recorded; it is not acknowledged.
#[instrument(level = "debug", skip_all, fields(task = %delivery.message.task))]
async fn record_delivery(conn: &mut Connection, delivery: &Delivery) -> Result<Position> {
    let position = delivery.position()?;
    record(
        conn,
        &Recorded {
            seq: position.seq,
            at_ms: position.at_ms,
            message: &delivery.message,
        },
    )
    .await
    .with_context(|| format!("Unable to record stream sequence {}", position.seq))?;
    if let Err(error) = delivery.ack().await {
        eprintln!(
            "Recorded stream sequence {} but the bus did not take the acknowledgement: {error:#}",
            position.seq
        );
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use std::time::Instant;
    use std::{env, fs, process};

    use super::*;

    use crate::message::{Kind, Message};
    use crate::store::read;

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("agent-bus-record-{name}-{}", process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn no_bus() -> Result<BusAccess> {
        bail!("no bus in this test")
    }

    async fn record_one(path: &Path, task: &str) {
        let store = Store::open(path).await.unwrap();
        let message = Message {
            task: task.to_owned(),
            from: "host".to_owned(),
            to: "dev".to_owned(),
            kind: Kind::Task,
            text: "do it".to_owned(),
            usage: None,
            task_usage: None,
            github: false,
            model: None,
        };
        let recorded = Recorded {
            seq: 1,
            at_ms: 1000,
            message: &message,
        };
        record(&mut store.connect().unwrap(), &recorded)
            .await
            .unwrap();
    }

    async fn tasks(record: &Record) -> Vec<String> {
        read::history(record.conn(), None, None, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|row| row.task)
            .collect()
    }

    #[tokio::test]
    async fn with_the_bus_down_a_command_answers_from_the_record_and_says_so() {
        let dir = scratch("down");
        let paths = Paths::in_dir(&dir);
        record_one(&paths.record, "t1").await;

        let record = Record::open(&paths, no_bus()).await.unwrap();
        assert_eq!(tasks(&record).await, ["t1"]);
        assert!(
            matches!(record.catch_up(), CatchUp::Failed(error) if error.to_string().contains("no bus")),
            "{:?}",
            record.catch_up()
        );
        let warning = record.behind().unwrap();
        assert!(warning.contains("not caught up"), "{warning}");
        assert!(warning.contains("no bus"), "{warning}");

        // An unreachable bus is the same: its CA is missing, so it is not even dialled.
        let unreachable = BusAccess {
            server: Endpoint {
                url: "tls://127.0.0.1:1".to_owned(),
                ca: dir.join("missing-ca.pem"),
            },
            seed: "not a seed".to_owned(),
        };
        drop(record);
        let record = Record::open(&paths, Ok(unreachable)).await.unwrap();
        assert!(matches!(record.catch_up(), CatchUp::Failed(_)));
        assert_eq!(tasks(&record).await, ["t1"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_open_record_holds_the_lock_and_a_closed_one_lets_go() {
        let dir = scratch("lock");
        let paths = Paths::in_dir(&dir);
        let record = Record::open(&paths, no_bus()).await.unwrap();
        assert!(try_lock_exclusive(&paths.lock).unwrap().is_none());
        drop(record);
        assert!(try_lock_exclusive(&paths.lock).unwrap().is_some());
        // A second command, in turn, opens the same record.
        let record = Record::open(&paths, no_bus()).await.unwrap();
        assert_eq!(tasks(&record).await, Vec::<String>::new());
        drop(record);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_held_record_times_out_naming_its_holder() {
        let dir = scratch("held");
        let paths = Paths::in_dir(&dir);
        let held = Record::open(&paths, no_bus()).await.unwrap();
        let wait = Duration::from_millis(300);
        let started = Instant::now();
        let error = lock_within(&paths.lock, wait).await.unwrap_err();
        let elapsed = started.elapsed();
        assert!(elapsed >= wait, "gave up after {elapsed:?}");
        assert!(elapsed < wait * 5, "waited {elapsed:?}");
        let error = format!("{error:#}");
        assert!(
            error.contains(&format!("process {}", process::id())),
            "{error}"
        );
        assert!(error.contains("record.lock"), "{error}");
        drop(held);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn messages_not_recorded_are_counted_on_one_line_by_reason() {
        assert_eq!(dropped_summary(&Tally::default()), None);
        let recorded_only = Tally {
            recorded: 7,
            ..Tally::default()
        };
        assert_eq!(dropped_summary(&recorded_only), None);
        let both = Tally {
            recorded: 7,
            foreign_sender: 1400,
            not_messages: 3,
        };
        assert_eq!(
            dropped_summary(&both).unwrap(),
            "Acknowledged without recording: 1400 whose sender is not the subject's, 3 that are \
             not agent messages"
        );
        let one = Tally {
            not_messages: 2,
            ..Tally::default()
        };
        assert_eq!(
            dropped_summary(&one).unwrap(),
            "Acknowledged without recording: 2 that are not agent messages"
        );
    }

    #[tokio::test]
    async fn a_command_waits_its_turn_for_a_short_holder() {
        let dir = scratch("turn");
        let paths = Paths::in_dir(&dir);
        let held = Record::open(&paths, no_bus()).await.unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(200)).await;
            drop(held);
        });
        let lock = lock_within(&paths.lock, Duration::from_secs(5)).await;
        assert!(lock.is_ok(), "{lock:?}");
        release.await.unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }
}
