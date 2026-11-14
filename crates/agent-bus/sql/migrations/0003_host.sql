-- Schema version 3: the host's own usage and time, imported from Claude Code's transcripts,
-- the host's active work item, and the time buckets of each agent run. See "Host usage" and
-- "Time" in docs/architecture/AGENT_BUS.md.

-- One row per transcript file read: where reading stopped (`offset`, always just after a line
-- end) in which file (`inode`). `agent_id` is set for a subagent's transcript; `started_at_ms`
-- is the time of its first line, which charges a subagent whole to the item active then.
CREATE TABLE host_transcript (
    path TEXT PRIMARY KEY,
    project TEXT NOT NULL,
    session TEXT NOT NULL,
    agent_id TEXT,
    inode INTEGER NOT NULL,
    offset INTEGER NOT NULL,
    started_at_ms INTEGER,
    imported_at_ms INTEGER NOT NULL,
    skipped INTEGER NOT NULL
) STRICT;

-- One row per API response of the host, keyed by Claude Code's `message.id` across every
-- file: a resumed or forked session copies its history, and it is counted once. `at_ms` is
-- its earliest line, `last_at_ms` its latest; `session` is the session it was first seen in.
-- `cost_usd` is NULL when its model has no rate or it was not at the standard speed. `work`
-- is the item it is charged to (NULL: unattributed); `pinned` is set by `work reassign`, and a
-- pinned row is never attributed again.
CREATE TABLE host_usage (
    message_id TEXT PRIMARY KEY,
    session TEXT NOT NULL,
    agent_id TEXT,
    at_ms INTEGER NOT NULL,
    last_at_ms INTEGER NOT NULL,
    model TEXT NOT NULL,
    speed TEXT NOT NULL,
    input INTEGER NOT NULL,
    output INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    cache_write_5m INTEGER NOT NULL,
    cache_write_1h INTEGER NOT NULL,
    cost_usd REAL,
    work INTEGER REFERENCES work_item (id),
    pinned INTEGER NOT NULL
) STRICT;
CREATE INDEX host_usage_session ON host_usage (session, at_ms);
CREATE INDEX host_usage_work ON host_usage (work);

-- The host's time, as the transcripts give it: each prompt (`prompt` from the user, `notice`
-- from Claude Code) at `start_ms`, and each tool call from its start to its result, by its
-- bucket (`work`, `testing` or `excluded`); `end_ms` is NULL until the result is read. Keyed
-- by the line's `uuid` or the call's ID, so a copied history is counted once. The buckets are
-- worked out when reported, from these and the responses' times.
CREATE TABLE host_span (
    id TEXT PRIMARY KEY,
    session TEXT NOT NULL,
    agent_id TEXT,
    kind TEXT NOT NULL,
    start_ms INTEGER NOT NULL,
    end_ms INTEGER
) STRICT;
CREATE INDEX host_span_session ON host_span (session, start_ms);

-- The host's active work item from `from_ms` on: for one Claude Code session, or every session
-- when `session` is NULL. `work` NULL is none (`work clear`). `source` is `use`, `clear` or
-- `send`; `task` is the task a `send` tagged.
CREATE TABLE host_focus (
    id INTEGER PRIMARY KEY,
    from_ms INTEGER NOT NULL,
    session TEXT,
    work INTEGER REFERENCES work_item (id),
    source TEXT NOT NULL,
    task TEXT
) STRICT;

-- Facts about the import: `installed_at_ms`, the time of the first import. Lines from before it
-- are not imported.
CREATE TABLE host_meta (
    key TEXT PRIMARY KEY,
    value INTEGER NOT NULL
) STRICT;

-- The time buckets of an agent run, from a guest that reports them; NULL from older guests.
-- Its waiting on the user is not here: it is each question's time to its answer, from the
-- messages.
ALTER TABLE run ADD COLUMN work_ms INTEGER;
ALTER TABLE run ADD COLUMN testing_ms INTEGER;
