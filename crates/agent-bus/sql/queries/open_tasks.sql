-- The tasks still open: sent, running or waiting on a question, oldest first, with their item.
SELECT task.id, task.agent, task.state, task.sent_at_ms, task.last_at_ms, task.result,
    task_work.work
FROM task
LEFT JOIN task_work ON task_work.task = task.id
WHERE task.state IN ('sent', 'running', 'question')
ORDER BY task.first_seq
