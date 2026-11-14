-- The messages of task ?1 that agent ?2 sent or received, oldest first.
SELECT task, from_agent, to_agent, kind, text, model, github, usage, task_usage, work, ended
FROM message
WHERE task = ?1 AND (from_agent = ?2 OR to_agent = ?2)
ORDER BY seq
