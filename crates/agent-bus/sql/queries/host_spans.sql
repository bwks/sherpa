-- The prompts and tool calls of the host's main conversations (not subagents).
SELECT session, kind, start_ms, end_ms FROM host_span WHERE agent_id IS NULL ORDER BY session, start_ms
