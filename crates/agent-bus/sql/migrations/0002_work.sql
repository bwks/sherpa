-- Schema version 2: work items. An item is one piece of work, named by its issues, pull
-- requests and branches, and the tasks that worked on it. See "Work items" in
-- docs/architecture/AGENT_BUS.md.

-- The two new fields of a message, as received, JSON like `usage`: a task's `work` tags and an
-- outcome's `ended`. Turso takes ADD COLUMN on a STRICT table; rows recorded before have NULL.
ALTER TABLE message ADD COLUMN work TEXT;
ALTER TABLE message ADD COLUMN ended TEXT;

-- One row per item. `merged_into` is set when `link` merged the item into another: its refs and
-- tasks moved there, and the row stays to show what was merged.
CREATE TABLE work_item (
    id INTEGER PRIMARY KEY,
    created_at_ms INTEGER NOT NULL,
    merged_into INTEGER REFERENCES work_item (id)
) STRICT;

-- The names of an item. A ref names one item: (`repo`, `kind`, `ref`) is the key. `kind` is
-- `issue`, `pull_request` or `branch`; `ref` is the number or the branch. `source` is how it
-- was learned: `brief` (the task's tag), `guest` (an outcome's `ended`), `parsed` (an
-- outcome's Generation line) or `manual` (`link`).
CREATE TABLE work_ref (
    repo TEXT NOT NULL,
    kind TEXT NOT NULL,
    ref TEXT NOT NULL,
    work INTEGER NOT NULL REFERENCES work_item (id),
    source TEXT NOT NULL,
    at_ms INTEGER NOT NULL,
    PRIMARY KEY (repo, kind, ref)
) STRICT;
CREATE INDEX work_ref_work ON work_ref (work);

-- Every ref a task's messages named, kept whether or not it names the task's item: a ref that
-- names another item is a conflict, shown by `work show`. `seq` is the message that named it
-- first.
CREATE TABLE task_ref (
    task TEXT NOT NULL,
    repo TEXT NOT NULL,
    kind TEXT NOT NULL,
    ref TEXT NOT NULL,
    source TEXT NOT NULL,
    seq INTEGER NOT NULL,
    PRIMARY KEY (task, repo, kind, ref)
) STRICT;
CREATE INDEX task_ref_ref ON task_ref (repo, kind, ref);

-- The item each task belongs to: one, so its cost is never split.
CREATE TABLE task_work (
    task TEXT PRIMARY KEY,
    work INTEGER NOT NULL REFERENCES work_item (id),
    source TEXT NOT NULL,
    at_ms INTEGER NOT NULL
) STRICT;
CREATE INDEX task_work_work ON task_work (work);
