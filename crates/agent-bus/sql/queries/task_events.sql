-- When each task in a work item was sent, answered, asked a question or reported an outcome
-- (not its progress), oldest first: what the host's work follows.
SELECT message.task, message.at_ms
FROM message
JOIN task_work ON task_work.task = message.task
WHERE message.kind IN ('task', 'answer', 'question', 'completed', 'failed', 'canceled')
ORDER BY message.seq
