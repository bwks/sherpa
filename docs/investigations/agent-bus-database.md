# Agent bus: an embedded database for state, costs and history

> Status: investigation, 2026-11-11, for #555; revised the same day after the user's decisions
> (Turso, no other SQLite library; then no recorder daemon and no socket). Phase 1 is
> implemented on `feat/555-agent-bus-db`, and how it works is in [`../architecture/AGENT_BUS.md`](../architecture/AGENT_BUS.md) ("The record",
> "Files and state"). The bus's earlier research is in [`agent-bus.md`](agent-bus.md). Related:
> #527 (a listener that never reads the host inbox), #494 (open agent-bus work).

## Question

agent-bus keeps its history in the `AGENTS` stream, which drops messages after 7 days, and in
per-machine files pruned to the same 7 days (the host's `listen-<agent>.jsonl`, the guest's
`results.jsonl`) or 30 days (the guest's `generation.toml`). #555 asks whether an embedded
database should keep that history, starting with task costs, and answers nine questions: seven
in the issue, two in its comment about running one host session per guest agent.

## Decisions taken

- **Turso.** The user chose Turso (`turso` =0.8.3, already in `Cargo.lock` for `crates/db`) and
  ruled out `rusqlite` and every other SQLite library. The first version of this note recommended
  `rusqlite`; its measurements are kept below as evidence, and the design is now built around
  Turso's one-process lock.
- **No experimental Turso modes.** The record is opened as `crates/db` opens its database.
- **No daemon, no socket.** Phase 1 was first built with `agent-bus record`, a long-running
  recorder answering queries on a Unix socket, with commands opening the record themselves only
  when no recorder ran. The user dropped the recorder and the socket: every command takes the
  lock, catches up and queries, and that is the only path. See
  [Reading the record](#reading-the-record).

## What was looked at

- `crates/agent-bus` on main at `05aa55de` (0.5.33): `channel.rs`, `listen.rs`, `report.rs`,
  `config.rs`, `message.rs`, `main.rs`, `files.rs`, and `guest/` (`run.rs`, `ledger.rs`,
  `generation.rs`, `pull_request.rs`).
- `docs/architecture/AGENT_BUS.md`, "Usage and cost", "Pull request Generation section" and
  "Files and state".
- `crates/db` (`build.rs`, `src/connect.rs`, `src/schema/apply.rs`), `docs/architecture/DATABASE.md`
  and [`turso-migration.md`](turso-migration.md).
- `turso_core` 0.8.3's file lock (`io/unix.rs`, `UnixFile::lock_file`).
- #527, and PRs #562 and #563, which both took version 0.5.30.
- The state directory of guest `dev02`, for sizes: `results.jsonl` held 71 `result` events from
  28 tasks over about 35 hours, 297 KB (about 4 KB per event, mostly the result text);
  `generation.toml` held 12 tasks in 2 KB.
- A throwaway crate outside the repository (`/tmp`, removed after) comparing `turso` 0.8.3 (as
  `crates/db` pins it, default features off) with `rusqlite` 0.42 (`bundled`). See
  [Measurements](#measurements).

## Findings

### What the host sees today

- Every message is published to `agents.<to>.<from>` and kept in one stream on `agents.>` for
  7 days (`channel.rs:36-41`, `channel.rs:151-157`). The stream has the default limits retention
  (no `retention` is set), so any number of consumers can each read every message.
- The host inbox is a durable consumer named `host`, filtered on `agents.host.>`, with
  `max_waiting = 1` (`channel.rs:613-624`); `Channel::inbox` refuses a second reader
  (`channel.rs:264-290`). `listen` journals, prints and then acknowledges each message
  (`listen.rs:172-196`).
- `usage` and `show` read the host inbox's subjects through an ephemeral, unacknowledged consumer
  (`channel.rs:303-327`, `main.rs:532-556`), so they see only guest-to-host messages: outcomes,
  progress and questions. The task text, the requested model and `--github` are on the
  host-to-guest subjects (`agents.<guest>.host`), which nothing reads back today.
- The host key may use the whole JetStream API and acknowledge on any consumer of the stream
  (`config.rs:134-143`). Creating another durable consumer needs no permission change. It may
  subscribe only under its own reply prefix, `_INBOX_<key>.>`.
- A message has no timestamp of its own (`message.rs:34-56`); the stream's publish time is in
  each delivery's metadata. There is no "started" message: a task's first `progress` is the
  nearest thing.
- Outcome messages carry the run's `usage` (with per-model entries, subagents included) and the
  task's cumulative `task_usage`. Priming runs are not messages of their own: they are folded into
  `task_usage` only (AGENT_BUS.md, "Usage and cost"), so the sum of a task's runs can be below its
  total, by the priming it triggered.
- No message carries the branch, issue or pull request. The guest knows the branch after each run
  (`run.rs:1059-1108`) and puts it, with the pull request URLs it updated, only into the first line
  of the outcome text (`pull_request.rs:81`, "Generation section for branch ...").

### What landed for the Generation section

The issue describes `feat/agent-bus-generation` as work in progress. It is on main
(`76c150ca`, `66fa12c2`) as the issue describes it:

- a ledger per guest VM, `generation.toml`, shared by every agent name on that VM, kept 30 days
  (`guest/ledger.rs:1-30`);
- after each run of a `--github` task the guest rewrites the section of every open pull request
  of the branch, in this checkout and in `../sherpa-docs` (`run.rs:1059-1108`);
- the rows are also written as TOML in a hidden comment, `<!-- agent-bus:generation-data ... -->`
  (`guest/generation.rs:22`), which is what is read back, so a guest keeps the rows of tasks run
  by another VM's guest.

### Turso, as `crates/db` uses it, holds the file for one process

`crates/db` is Sherpa's schema and depends on `shared`; agent-bus must not depend on it. What
is reused is the approach: one SQL file per statement, numbered migrations under
`PRAGMA user_version`, every statement prepared against the migrated schema. `crates/db/build.rs`
is specific to that crate (`SCHEMA_FILES`, fixed paths, `build.rs:25-43`).

The deciding property is how many processes can open the file. Measured with `turso` 0.8.3 and
the builder `crates/db` uses:

| Second process, while the first holds the file open and writes | Result |
|---|---|
| `Builder::new_local` (as `crates/db`) | Refused: "File is locked by another process" |
| `read_only(true)` | Opens, but reads a snapshot from when it opened: a row the writer added 4 s later was never seen in three more reads over 9 s |
| `experimental_multiprocess_wal(true)` on both, reader also read-only | Works: the reader saw the new row on its next read |
| `rusqlite` 0.42 with WAL, both processes | Works (SQLite's documented behaviour; one read checked) |

[`turso-migration.md`](turso-migration.md) found the same exclusivity with the `sqlite3` CLI.
The lock is a POSIX record lock (`fcntl` `F_SETLK`, `turso_core` `io/unix.rs:278-300`). POSIX
record locks belong to the process, not to the open file: a second open of the same file in the
same process is not refused, and closing either releases the process's lock. Its only signal is
the error text. So agent-bus does not use Turso's lock to decide who opens the record; it takes
its own `flock` on `record.lock` first (see the design below), and Turso's lock is a second
guard.

### Measurements

The throwaway crates were built cold and alone, in release, stripped, on the 12-core dev guest
(rustc 1.99.1). They are upper bounds for a crate that has neither yet; they are not the
increment on this workspace.

| | `turso` =0.8.3, no default features (+ `tokio`) | `rusqlite` 0.42, `bundled` |
|---|---|---|
| Crates in the dependency tree (normal dependencies, not counting the probe) | 233 | 9 |
| Cold release build | 418 s | 104 s |
| Binary | 18.1 MB | 2.4 MB |

In this workspace, `turso` is already in `Cargo.lock` and built for `sherpad`, so using it in
agent-bus adds no crate to the lock file, and shares the build with `crates/db` in a full
workspace build. The cost lands on the guests: they build only agent-bus
(`start-guest.sh:73`, `cargo build --release -p agent-bus`), never open the record, and now
compile and link Turso anyway. A cargo feature that leaves the record out of the guest's build
would remove that; it is listed for phase 2, not done.

### The host inbox, #527 and a second consumer

#527: a running `listen host` sometimes never pulls, while the bus holds the outcome. The cause is
not known. A database fed by `listen` would inherit that: when the listener stalls, the record
stalls. A separate durable consumer does not compete with the inbox. It gets its own copy of
every message, and the inbox keeps its one reader.

That consumer reads `agents.>`, not `agents.host.>`, so it also records what the host sent
(the task text, its model and `--github`, answers and cancels). Its name must not be an agent
name: agent names are lowercase letters, digits and hyphens (`message.rs:185-193`), so `_record`
can never collide with an inbox.

It shares the pull code with `listen` (`Inbox::next`), so if #527 is in that code, catching up
can stall too. The difference is that a stall then delays the record and loses nothing (while the
stream still holds the message): its position is durable, and acknowledging after the row is
committed makes it at-least-once, while the stream sequence as the key makes it exactly-once in
the table. Its `num_pending` is a lag figure that `agent-bus agents` can show (phase 2), and a lag
that grows while the host inbox's does not (or the reverse) narrows #527 down. With no
long-running reader it also grows between commands by design, so the figure to compare is the
one right after a command.

**Catch-up stopped at 200 messages (v0.5.34, fixed on `fix/agent-bus-record-catch-up`).** On
the first live deploy, `_record` had 6,085 messages waiting and each command recorded exactly 200
and called the record current, so `usage`, `show` and `history` answered from a stale record
without saying so. Cause, proven against a throwaway bus: async-nats sent its next pull of 200
once 100 were delivered, the bus dropped it without a reply because `max_waiting = 1` was taken
by the open pull, the client waited for messages that never came, and catching up took 2 s of
silence as "caught up". Fix: readers pull one message at a time, catching up reads until the
consumer's pending count is 0 and says it is behind when the bus stops delivering first, and
messages not taken are counted on one line. 6,050 messages now take one command, 28.5 s (25.1 s
of it commits), where before each command took about 3 s for 200 (30 commands). The same
overlapping pull may be behind #527 (`listen` inboxes also have `max_waiting = 1`); not proven.

Follow-up on #527 (2026-11-12, against a throwaway bus, nats-server 2.15.2). On an inbox
consumer, a pull sent while another waits is answered with 409 `Exceeded MaxWaiting` when the
request has no `idle_heartbeat`, and not at all when it has one (no 409, no heartbeat, no
expiry), from the same connection or another. async-nats 0.52's pull stream swallows that 409
and pulls again at once, printing nothing. A lone listener on 1-message pulls was not stalled
by 70 s idle, a bus restart or a 20 s pause; each message came in about 50 ms. The 30-minute
silent listeners of #527 were not reproduced. Readers now use their own pull loop that reports
refusals and silences (`src/pull.rs`; see "One reader per inbox" in
`docs/architecture/AGENT_BUS.md`).

The print and acknowledgement path, checked against the host's journal (2026-11-12). Task
359ab3b7's completion was journaled once, at 2026-11-10T08:06:09Z (`at_ms` 1794297969659),
seconds after its last progress line, so a listener pulled it promptly. Selecting the journal on
`message.task == "359ab3b7"` finds that one completion entry and no other. The entry matching
"359ab3b7" journaled at 2026-11-12T08:44:44Z is the completion of task 152443bd, whose text quotes
the ID: a different message, not a redelivery. A redelivery would have meant the message was
never acknowledged: a listener that journals and then fails to print exits without
acknowledging, and the next listener journals the message again after the 30 s acknowledgement
wait (test `a_message_a_listener_cannot_print_is_delivered_again_and_journaled_twice`). A stdout
pipe nobody reads blocks `print` once the pipe buffer (64 KiB) is full, which would also stop the
acknowledgement and show up the same way.

Conclusion: the listener journaled, printed and acknowledged 359ab3b7's completion; `print`
returned, so the line was written to the listener's stdout. The loss was after that, in the
Monitor pipeline that turns the listener's stdout into notifications for the host agent, not in
`agent-bus` or the bus. The 30-minute silent listeners of #527 were therefore not an `agent-bus`
fault; the reader changes above make any stall in pulling visible on stderr rather than silent.

### Keeping the stream as the history

The option the first version skipped: raise the `AGENTS` stream's `max_age` (to a year, say) and
keep reading the stream as `usage` and `show` do today.

**Size.** Not measured on the host's stream; an estimate from dev02. A task is about 15 KB on the
stream: the task message (1 to 3 KB), its progress lines (up to 300 characters each, tens per
task) and one to three outcomes of about 4 KB each with their usage. At today's rate (about 20
tasks a day per guest, two guests) that is about 0.6 MB a day, about 220 MB a year, before
JetStream's per-message overhead. `nats stream info AGENTS` on the host gives the real figure.

**What it solves.** History past 7 days, for `usage` and `show` as they are. Per-agent sessions
too, to be fair: a consumer filtered on `agents.host.<agent>` follows one agent from the stream
alone.

**What it does not.**

- **Every read scans everything.** `usage` reads the whole kept stream through an ephemeral
  consumer on every call. A year is about 220 MB per call; a report by model or by week is
  client-side aggregation over all of it, each time.
- **No joins.** Issues, pull requests and branches are not in the messages, and the stream has no
  place for a link added later (`agent-bus link`), or for anything derived.
- **Not a durable record of its own.** The history would live in the Docker volume
  `sherpa-agent-bus-data`, which `docker volume rm`, a rebuilt host or a reset of the bus loses,
  and which nothing backs up. The record is one file in the state directory, to copy or back up
  like any other.
- **One knob for three things.** `RETENTION` (`channel.rs:41`) also sets the status bucket's
  `max_age` and how long the listener's journal and the guest's `results.jsonl` keep lines; a year
  of `results.jsonl` (about 4 KB per run) would need its own setting first. And
  `create_inboxes` uses `get_or_create_stream`, which does not change an existing stream's
  `max_age`: raising it needs an update path.

**Verdict.** Rejected as the history store, and not adopted as a stopgap: phase 1 replaces it on
this branch. A smaller raise does have a use, as the record's buffer: with no long-running
writer, the stream holds every message until a command catches the record up, and more than
7 days with no command loses what the stream dropped. Raising `max_age` to 30 days (about 20 MB at the estimate above),
with `RETENTION` split from the file retention, is in phase 2.

### Reading the record

Turso holds the file for one process, so something has to decide which process that is, and
how every other command reads. Four options were weighed.

**A. A recorder holds the file; reads go to it over the bus.** NATS request/reply on an
internal subject, as the host suggested. It works, but the bus is the wrong transport for it:

- The host key may subscribe only under `_INBOX_<key>.>` and publish only to `agents.>`, the
  JetStream API and acknowledgements. The recorder would need to subscribe to a request subject
  and publish replies, so the host's permissions widen, and a changed `nats.conf` means a
  `bus start` or reload before it works.
- The record is a local file, but reading it would fail whenever the bus is down, which is
  exactly when a record independent of the bus is wanted.
- A reply is capped by `max_payload` (1 MB by default): `show` of a long task or a long `history`
  would need paging.
- Every client of the queries is on the host; nothing gains from routing them through the bus.

**B. `experimental_multiprocess_wal` on every open.** Each command opens the file. It worked in
the probe, but it is a mode Turso calls experimental in a pinned pre-1.0 release, it must be on
for every opener, `crates/db` does not use it, and its read-only cousin returned stale data
silently, which is the failure #527 is about. Once there is no daemon it also buys almost
nothing: no process keeps the file open, each command holds it for milliseconds (seconds while
catching up a large backlog), so concurrent opens are rare and a short wait on the lock covers
them. That small gain does not pay for an experimental mode in a pinned pre-1.0 release.
Rejected; revisit when Turso drops "experimental".

**D. A recorder daemon with a query socket.** Built first on this branch, then dropped by the
user: `agent-bus record` held the lock, recorded from `_record` as messages came, and answered
`usage`, `show` and `history` over `<state>/record.sock` (one JSON query line in, one JSON reply
line out), with C as the fallback when no recorder ran. It avoided A's problems (no permission
change, no payload cap, answers while the bus is down), but:

- The server, the wire types (`Query`, `Reply`), the socket file and its stale-socket handling,
  and the retry loop for "lock held, nobody answering" existed only because Turso lets one
  process open the file. None of it was the record's own logic.
- It made two paths to every answer (through the recorder, or opening the record), which had to
  be kept identical and were tested twice.
- It gained nothing the stream does not already give: `_record` is durable, so the stream
  buffers every message until a command catches up. A recorder only moved the catch-up cost
  from the command to a background process, and a command after a busy stretch costs a moment.
- It was one more long-running process for the host to start, watch and restart.

**C. Each command opens the file, catches up, and lets go.** Chosen, as the only path:

- Whoever holds an exclusive `flock` on `<state>/record.lock` is the only process that opens
  `<state>/agent-bus.db`. Turso's own POSIX lock is per process (a second open in the same
  process is not refused, closing either releases it), so it is a second guard, not the arbiter.
- `usage`, `show` and `history` each: connect to the bus (before the lock, so a slow refusal
  does not hold it), take the lock, open the record, catch up from `_record` (commit, then
  acknowledge; the stream sequence is the key), run the query, close the record, release the
  lock, and then print.
- When the bus cannot be reached, or `AGENT_BUS_SEED` is missing, the command answers from what
  is recorded and says it did not catch up.
- The lock's holder writes its PID into `record.lock`. A command waits up to 5 s for the lock,
  then fails naming that PID. Commands hold it for milliseconds, or a few seconds while catching
  up a large backlog, so a longer wait would only hide a stuck process.
- No query protocol: a command calls the `store::read` functions on its own connection.

**`follow --agent` (phase 3).** A loop on the same path: take the lock, catch up, read the rows
to or from the agent with a sequence above the last one printed, release the lock, print them,
wait a moment, repeat. A restart takes `--after <seq>` or `--since <d>`. Each turn holds the lock
for milliseconds, so other commands interleave with it. Republishing recorded messages on a
per-agent NATS subject was considered and dropped: core NATS loses what is published while the
follower is away, so the follower needs the cursor query anyway, and the host key would need
subscribe permissions.

### Checking the SQL: a test, not a `build.rs`

`crates/db` prepares every statement in `build.rs`, so a statement that does not fit the schema
fails `cargo build`. agent-bus does it in a unit test instead (`store::database::tests`): it
applies every migration to an in-memory database, prepares every query, checks that every file
under `sql/queries/` is listed and that the migrations are numbered `0001`, `0002`, ... without a
gap. The reason is the guest's build: a `build.rs` needs Turso as a build dependency, and in a
release build build dependencies are compiled with the `build-override` profile, so Turso would
compile twice on every guest for eight statements. The cost is that a broken statement fails
`cargo test` rather than `cargo build`; CI runs both.

### Versions and migration numbers

#562 and #563 both took 0.5.30 because each pull request bumped the version then. Since #571 and
#574, pull requests do not change the version: a bump pull request does, and the `Version` check
fails any other (AGENTS.md, "Versions and Releases"; `docs/architecture/RELEASE.md`, "The Version
check"). That collision cannot happen again, and the database need not track versions.

Migration numbers can still collide: two open branches each adding
`crates/db/sql/migrations/0017_*.sql`. `crates/db/build.rs` refuses a duplicate or a gap
(DATABASE.md, "Versions"), so the second to merge fails its build once it is rebased on the
first. A claim table in the record could hold "migration 0017 is held by task X for #N", but only
for work sent through the bus (not the user's own branches), and only the host could write it,
so a guest would need a round trip before picking a number. The cheaper rule is for the host to
name the number in the brief when two open tasks touch `crates/db`. Not worth a table yet.

## Answers to the questions

1. **Where it lives.** One record on the host, `<state>/agent-bus.db` in the host's state
   directory, 0600 with its `-wal` file. The host is the only place that sees every agent's
   messages. Guests keep their files as they are for now (see 6).
2. **Turso or SQLite.** Turso, the user's decision, opened as `crates/db` opens its database, with
   no experimental mode. Its one-process lock is handled by giving the file to one process at a
   time (see [Reading the record](#reading-the-record)). The approach of `crates/db` carries
   over: SQL files, numbered migrations, `user_version`, a newer record refused; the statements
   are checked by a test rather than `build.rs`.
3. **Who writes.** One process at a time: the command holding `record.lock`, which catches the
   record up from the `_record` consumer before it reads. `_record` has `max_waiting = 1` like
   the inboxes and `max_ack_pending = 1`, so messages are recorded in stream order. There is no
   long-running writer. The host inbox keeps its one reader, `listen`; recording does not touch
   it.
4. **Retention.** Keep everything at first; it is small. Figures are a few hundred bytes per run.
   The bulk is text: about 4 KB per outcome on dev02, plus progress lines (up to 300 characters
   each). At today's rate that is tens of MB a year. Add a prune of progress text after 30 days if
   the file grows; never prune figures.
5. **Backfill.** Free: `_record` is created with "deliver all", so the first catch-up records
   what the stream still holds (7 days). Older figures exist only in the guests'
   `generation.toml` (30 days, no text) and in the hidden data comments of pull requests; an
   import of those is optional and can come later.
6. **The Generation work.** It landed as the issue describes (per-VM ledger, hidden TOML). Moving
   it to the host would need two things: the branch in outcome messages, and the host editing pull
   request bodies with the GitHub write token in a long-running process, which today only reaches
   a guest per `--github` task. The hidden TOML works and keeps each pull request self-contained.
   Keep both for now; the record holds the same figures for reporting. Whether the host takes over
   the section is a later decision (phase 4).
7. **Reporting.** `usage` reads the record and gains `--since` (phase 1), then `--issue`, `--pr`,
   `--branch` and `--by agent|model|week` (phase 2); `history` lists tasks with state, model,
   cost and the start of the prompt (phase 1); `show` works past 7 days and shows both directions
   (phase 1); `follow --agent` serves a per-agent session (phase 3); `open` lists tasks with no
   final outcome or an unanswered question, for resuming after a context reset (phase 2). All
   terminal tables (`tabled`), with `--json` where a script wants it. No web page.
8. **Following one agent from the database.** Yes, with a cursor, on the same path as every
   command: see `follow --agent` under [Reading the record](#reading-the-record).
9. **What sessions share.** Versions: nothing, the bump rule settled it. Migration numbers: the
   build refuses duplicates; the host names the number in the brief when two tasks need one. A
   claim table can come later if collisions keep happening.

## Recommendation

- **Where.** `<state>/agent-bus.db` on the host, 0600, with its `-wal` file.
- **Dependency.** `turso` through a new workspace dependency, `=0.8.3` without default features,
  which `crates/db` now uses too. No other crate.
- **One path, no daemon.** Every command that needs the record (`usage`, `show`, `history`, and
  later `follow`) takes `<state>/record.lock` (waiting up to 5 s, then failing with the holder's
  PID), opens the record, catches it up from the durable consumer `_record` (filter
  `agents.>`, deliver all, `max_waiting = 1`, `max_ack_pending = 1`, created by `bus start`
  beside the inboxes), runs its query, closes the record and releases the lock. Each message is
  one transaction: the message row, the task's row, the run and its models; commit, then
  acknowledge. The stream sequence is the key, so a redelivery changes nothing.
- **Bus down.** With the bus unreachable or the seed missing, a command answers from what is
  recorded and says it did not catch up.
- **Queries.** Plain SQL files in `crates/agent-bus/sql/` (`migrations/0001_record.sql`,
  `queries/*.sql`), `PRAGMA user_version`, a record newer than the binary refused; a unit test
  prepares every statement against the migrated in-memory schema.

### Schema (phase 1, as built)

Times are milliseconds since the Unix epoch. Tables are `STRICT`. The SQL is
`crates/agent-bus/sql/migrations/0001_record.sql`.

**`message`**: every message read from `_record`, the log everything else derives from.

| Column | Contents |
|---|---|
| `seq` | The stream sequence. Primary key; a redelivery is ignored (`ON CONFLICT DO NOTHING`). |
| `at_ms` | The stream's publish time. |
| `task`, `from_agent`, `to_agent`, `kind` | As in the message. Indexed on (`task`, `seq`), (`from_agent`, `seq`) and (`to_agent`, `seq`), the last two for `follow`. |
| `text` | The message text. |
| `model`, `github` | As in the message. |
| `usage`, `task_usage` | The JSON objects as received, when present. |

**`task`**: one row per task, updated from each of its messages (`store::task::apply`, no I/O).

| Column | Contents |
|---|---|
| `id` | The task ID. Primary key. |
| `agent`, `requester` | The agent working on it and the one that sent it, from the first message seen. |
| `first_seq`, `last_seq`, `last_at_ms` | The task's oldest and newest message recorded, and the newest's time. |
| `requested_model`, `github`, `prompt`, `sent_at_ms` | From the `task` message. |
| `started_at_ms` | The first `progress`. |
| `finished_at_ms` | The last `completed`, `failed` or `canceled`. |
| `state` | `sent`, `running`, `question`, `completed`, `failed` or `canceled`. |
| `model` | The model the agent says it ran on, from its last outcome that named one. |
| `result` | The text of the last question or outcome. |
| `questions`, `answers` | Counts, for round trips. |
| `runs`, `cost_usd`, `input`, `output`, `cache_creation`, `cache_read`, `num_turns`, `duration_ms`, `duration_api_ms` | The last `task_usage`; NULL until an outcome carries one. Priming is included here and only here. |

What a message says about where the task is (state, result, model, usage) counts only when it is
newer than every message applied before, so one delivered out of order cannot take the task back;
what it adds (the prompt, a count, the first progress) counts whatever its order. A task first
seen through a later message (sent before the stream's oldest message) has no prompt or sent time.

**`run`**: one row per outcome message that carried `usage`: `seq` (primary key, the message's),
`task`, `outcome` (the kind), `model`, the four token counts, `num_turns`, `duration_ms`,
`duration_api_ms`, `cost_usd`, `session_cost_usd`.

**`run_model`**: one row per model in a run's `usage.models`, subagents included: `run`, `model`,
`canonical_model`, `cost_basis`, the four token counts and `cost_usd`. Primary key (`run`,
`model`).

**`work_item`** (phase 2): which task worked on what.

| Column | Contents |
|---|---|
| `task` | References `task`. |
| `repo` | `owner/name`. |
| `kind` | `issue`, `pull_request` or `branch`. |
| `ref` | The number, or the branch name. |
| `source` | `brief` (given by the host with `send`), `guest` (reported in an outcome), `parsed` (read from an outcome's Generation line, for backfill) or `manual`. |
| `at_ms` | When it was recorded. |

Primary key (`task`, `repo`, `kind`, `ref`). Filling it needs two small message changes, both
optional fields that older peers ignore: `send --issue <n>` / `--pr <n>` puts a `work` list on the
`task` message, and the guest puts the branch and the pull requests it updated on each outcome
(the values `Engine::generation` already has, `run.rs:1097-1104`). For history from before,
catching up parses the "Generation section for branch" line, marked `parsed`. `agent-bus link <task>
--issue|--pr|--branch` covers the rest.

Left for later: questions with answer latency (derivable from `message` already), gate results,
review findings, agents and heartbeats (the `AGENT_STATUS` bucket is another stream), and claims.

### Plan

1. **Record (done on `feat/555-agent-bus-db`).** `turso` as a workspace dependency; `_record`
   created by `bus start`; `record.lock` and the one path (`record::Record::open`: lock with a
   5 s wait naming the holder, open, catch up, query, close); the `message`, `task`, `run` and
   `run_model` tables; backfill on first catch-up; `usage` (with `--since`), `show` (both
   directions) and a new `history` (`--agent`, `--since`, `--limit`, `--json`) reading the
   record. `Channel::history` and the ephemeral consumer are gone. The recorder daemon, its
   socket and its query protocol were built and then removed (option D).
2. **Work items and upkeep.** The `work` and branch fields on messages, `work_item`, backfill
   parsing, `link`, `usage --issue/--pr/--branch/--by`, `open`. `agents` shows the `_record`
   consumer's lag. `RETENTION` split from the file retention, and the stream's `max_age` raised
   to 30 days as the record's buffer between commands. A cargo feature that leaves the record
   out of the guest's build.
3. **Per-agent sessions.** `follow --agent <name> [--after <seq> | --since <d>]`, a loop on the
   same path: lock, catch up, read past the cursor, release, print, wait. The host agent setup
   moves from one `listen host` to one `follow` per session; `listen host` stays for anyone not
   using the record, and something must still drain the host inbox (a `listen` whose output is
   discarded, or catching up acknowledging the inbox too, decided then).
4. **Decide the Generation section.** With the branch and the figures on the host, decide whether
   the host writes the section (the write token on the host, the ledger and the hidden TOML gone)
   or the guest keeps doing it. Then the later tables: questions, gate results, review findings,
   agents.

## Draft follow-up issue

**Title:** agent-bus: work items, reports by issue/PR and per-agent sessions on the host record

**Body:**

> Follows phase 1 of `docs/investigations/agent-bus-database.md` (#555), which records every
> message on the bus in `<state>/agent-bus.db` (Turso). There is no recorder process: each
> command that reads the record (`usage`, `show`, and the new `history`) takes `record.lock`,
> catches the record up from the durable consumer `_record`, queries it and lets go.
>
> **Phase 2, work items and upkeep**
>
> - Optional `work` on `task` messages (`send --issue <n> --pr <n>`), and the branch and updated
>   pull requests on outcomes from the guest; older peers ignore both.
> - Table `work_item` (migration `0002`); backfill from the outcome's "Generation section for
>   branch" line, marked `parsed`; `agent-bus link <task> --issue|--pr|--branch`.
> - `usage --issue/--pr/--branch` and `--by agent|model|week`; `open` for tasks in flight or
>   waiting on an answer.
> - `agents` shows the `_record` consumer's lag beside the host inbox's (#527).
> - Split `RETENTION` from the file retention; raise the `AGENTS` stream's `max_age` to 30 days,
>   with an update path for an existing stream, since the stream is the record's only buffer
>   between commands.
> - A cargo feature so the guest's `cargo build -p agent-bus` leaves Turso out.
>
> **Phase 3, per-agent sessions**
>
> - `agent-bus follow --agent <name> [--after <seq> | --since <d>]` on the same path as every
>   command: take the lock, catch up, print what is past the cursor, release, wait, repeat. No
>   daemon and no socket.
> - Decide what drains the host inbox once sessions follow the record.
>
> **Not in this issue:** moving the Generation section to the host (phase 4), questions, gate
> results, review findings, agents' history, migration-number claims.
