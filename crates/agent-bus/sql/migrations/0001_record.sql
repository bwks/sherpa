-- Schema version 1: the record of every message on the bus, and what is derived from it.
-- Times are milliseconds since the Unix epoch. Token counts and durations are whole numbers;
-- costs are API-equivalent US dollars, not billed.

-- Every message read from the AGENTS stream. The stream sequence is the key, so
-- a message delivered again is recorded once. `usage` and `task_usage` are the JSON objects as
-- received.
CREATE TABLE message (
    seq INTEGER PRIMARY KEY,
    at_ms INTEGER NOT NULL,
    task TEXT NOT NULL,
    from_agent TEXT NOT NULL,
    to_agent TEXT NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    model TEXT,
    github INTEGER NOT NULL,
    usage TEXT,
    task_usage TEXT
) STRICT;
CREATE INDEX message_task ON message (task, seq);
CREATE INDEX message_from_agent ON message (from_agent, seq);
CREATE INDEX message_to_agent ON message (to_agent, seq);

-- One row per task, updated from each of its messages. `agent` works on the task and
-- `requester` sent it. A task first seen through a later message (sent before the stream's
-- oldest message) has no prompt and no sent time. The usage columns are the task's last
-- `task_usage`, priming runs included; they are NULL until an outcome carries one.
CREATE TABLE task (
    id TEXT PRIMARY KEY,
    agent TEXT NOT NULL,
    requester TEXT NOT NULL,
    first_seq INTEGER NOT NULL,
    last_seq INTEGER NOT NULL,
    last_at_ms INTEGER NOT NULL,
    requested_model TEXT,
    github INTEGER NOT NULL,
    prompt TEXT,
    sent_at_ms INTEGER,
    started_at_ms INTEGER,
    finished_at_ms INTEGER,
    state TEXT NOT NULL,
    model TEXT,
    result TEXT,
    questions INTEGER NOT NULL,
    answers INTEGER NOT NULL,
    runs INTEGER,
    cost_usd REAL,
    input INTEGER,
    output INTEGER,
    cache_creation INTEGER,
    cache_read INTEGER,
    num_turns INTEGER,
    duration_ms INTEGER,
    duration_api_ms INTEGER
) STRICT;
CREATE INDEX task_first_seq ON task (first_seq);
CREATE INDEX task_agent ON task (agent, first_seq);

-- One row per outcome message that carried a run's `usage`.
CREATE TABLE run (
    seq INTEGER PRIMARY KEY REFERENCES message (seq),
    task TEXT NOT NULL REFERENCES task (id),
    outcome TEXT NOT NULL,
    model TEXT,
    input INTEGER NOT NULL,
    output INTEGER NOT NULL,
    cache_creation INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    num_turns INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL,
    duration_api_ms INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    session_cost_usd REAL NOT NULL
) STRICT;
CREATE INDEX run_task ON run (task);

-- One row per model in a run's `usage.models`, subagents included.
CREATE TABLE run_model (
    run INTEGER NOT NULL REFERENCES run (seq),
    model TEXT NOT NULL,
    canonical_model TEXT NOT NULL,
    cost_basis TEXT NOT NULL,
    input INTEGER NOT NULL,
    output INTEGER NOT NULL,
    cache_creation INTEGER NOT NULL,
    cache_read INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    PRIMARY KEY (run, model)
) STRICT;
