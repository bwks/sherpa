-- The tasks `usage` reports: those with an outcome, sent by ?1, for the agent ?2 when it is not
-- NULL, active at or after ?3 when it is not NULL, only those with usage unless ?4, and only
-- those of item ?5 when it is not NULL. With each, its item and the time buckets its runs
-- reported (NULL when none did).
SELECT task.id, task.agent, task.model, task.state, task.sent_at_ms, task.last_at_ms,
    task.runs, task.cost_usd, task.input, task.output, task.cache_creation, task.cache_read,
    task.num_turns, task.duration_ms, task.duration_api_ms, task_work.work,
    (SELECT SUM(run.work_ms) FROM run WHERE run.task = task.id),
    (SELECT SUM(run.testing_ms) FROM run WHERE run.task = task.id)
FROM task
LEFT JOIN task_work ON task_work.task = task.id
WHERE task.requester = ?1
    AND task.state IN ('question', 'completed', 'failed', 'canceled')
    AND (?2 IS NULL OR task.agent = ?2)
    AND (?3 IS NULL OR task.last_at_ms >= ?3)
    AND (?4 OR task.runs IS NOT NULL)
    AND (?5 IS NULL OR task_work.work = ?5)
ORDER BY task.first_seq
