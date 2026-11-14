-- The tasks of item ?1, all items when ?1 is NULL, in the order they were first recorded.
SELECT task_work.work, task_work.source, task.id, task.agent, task.state, task.model,
    task.requested_model, task.prompt, task.last_at_ms, task.runs, task.cost_usd
FROM task_work
JOIN task ON task.id = task_work.task
WHERE ?1 IS NULL OR task_work.work = ?1
ORDER BY task.first_seq
