-- Record a prompt or the start of a tool call, unless it is recorded.
INSERT INTO host_span (id, session, agent_id, kind, start_ms, end_ms)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT (id) DO NOTHING
