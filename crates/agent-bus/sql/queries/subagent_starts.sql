-- When each subagent started: its transcript's first line.
SELECT agent_id, MIN(started_at_ms)
FROM host_transcript
WHERE agent_id IS NOT NULL AND started_at_ms IS NOT NULL
GROUP BY agent_id
