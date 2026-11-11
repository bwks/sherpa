-- The newest ?3 tasks, for the agent ?1 when it is not NULL, active at or after ?2 when it is not
-- NULL, newest first.
SELECT id, agent, state, requested_model, model, github, prompt, sent_at_ms, last_at_ms, runs,
    cost_usd
FROM task
WHERE (?1 IS NULL OR agent = ?1)
    AND (?2 IS NULL OR last_at_ms >= ?2)
ORDER BY first_seq DESC
LIMIT ?3
