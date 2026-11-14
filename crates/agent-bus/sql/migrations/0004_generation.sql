-- Schema version 4: the Generation sections the host keeps in pull request bodies. See "Pull
-- request Generation section" in docs/architecture/AGENT_BUS.md.

-- One row per pull request whose section the host has tried to write. `work` is its item when
-- last looked at. `state` is the pull request's (`OPEN`, `MERGED` or `CLOSED`) at the last
-- write: once it is `MERGED` or `CLOSED` the section is frozen. `section` is the text last
-- written, from its start marker to its end marker, and `written_at_ms` when; both are NULL
-- until a write succeeds. `last_error` is why the last attempt failed, NULL after a success,
-- and `attempted_at_ms` when the last attempt was: a failure is tried again only after the
-- debounce, unless the sync is forced.
-- The first sync also sets the import fact `generation_since_ms` in `host_meta`: a pull
-- request never written is only looked at when its item was active since then.
CREATE TABLE generation_pr (
    repo TEXT NOT NULL,
    number INTEGER NOT NULL,
    work INTEGER NOT NULL REFERENCES work_item (id),
    state TEXT,
    section TEXT,
    written_at_ms INTEGER,
    last_error TEXT,
    attempted_at_ms INTEGER,
    PRIMARY KEY (repo, number)
) STRICT;
