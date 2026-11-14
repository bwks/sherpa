-- The responses of the host's main conversations (not subagents), with their item.
SELECT session, at_ms, last_at_ms, work
FROM host_usage
WHERE agent_id IS NULL
ORDER BY session, at_ms
