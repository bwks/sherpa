-- Where reading the transcript ?1 stopped now.
INSERT INTO host_transcript (path, project, session, agent_id, inode, offset, started_at_ms,
    imported_at_ms, skipped)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT (path) DO UPDATE SET
    project = excluded.project,
    session = excluded.session,
    agent_id = excluded.agent_id,
    inode = excluded.inode,
    offset = excluded.offset,
    started_at_ms = excluded.started_at_ms,
    imported_at_ms = excluded.imported_at_ms,
    skipped = excluded.skipped
