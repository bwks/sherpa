-- The responses of the host's main conversations (not subagents), with their item and model.
SELECT session, at_ms, last_at_ms, work, model
FROM host_usage
WHERE agent_id IS NULL
ORDER BY session, at_ms
