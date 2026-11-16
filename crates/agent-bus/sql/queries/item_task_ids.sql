-- Every task the record has in a work item, with its item, in the order the tasks were first
-- recorded.
SELECT task_work.work, task_work.task
FROM task_work
LEFT JOIN task ON task.id = task_work.task
ORDER BY COALESCE(task.first_seq, 0), task_work.task
