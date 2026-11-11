-- The tasks `usage` reports: those with an outcome, sent by ?1, for the agent ?2 when it is not
-- NULL, active at or after ?3 when it is not NULL, and only those with usage unless ?4.
SELECT id, agent, model, state, runs, cost_usd, input, output, cache_creation, cache_read,
    num_turns, duration_ms, duration_api_ms
FROM task
WHERE requester = ?1
    AND state IN ('question', 'completed', 'failed', 'canceled')
    AND (?2 IS NULL OR agent = ?2)
    AND (?3 IS NULL OR last_at_ms >= ?3)
    AND (?4 OR runs IS NOT NULL)
ORDER BY first_seq
