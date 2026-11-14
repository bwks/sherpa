-- The refs a task named that name another item than the task's: those of item ?1's tasks, and
-- those naming item ?1 from other items' tasks.
SELECT task_ref.task, task_ref.repo, task_ref.kind, task_ref.ref, task_ref.source,
    task_work.work, work_ref.work
FROM task_ref
JOIN task_work ON task_work.task = task_ref.task
JOIN work_ref ON work_ref.repo = task_ref.repo AND work_ref.kind = task_ref.kind
    AND work_ref.ref = task_ref.ref
WHERE task_work.work <> work_ref.work AND (task_work.work = ?1 OR work_ref.work = ?1)
ORDER BY task_ref.seq
