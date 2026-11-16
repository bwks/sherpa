-- The kind and time of every message of the tasks one requester (the host) sent, by task,
-- oldest first: when each task was open.
SELECT message.task, message.kind, message.at_ms
FROM message
JOIN task ON task.id = message.task
WHERE task.requester = ?1
ORDER BY message.task, message.seq
