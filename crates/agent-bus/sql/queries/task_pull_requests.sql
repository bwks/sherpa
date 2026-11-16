-- The pull requests each task named that name the task's own item, with how and in which
-- message: the pull requests that may count it. One that names another item is a conflict and
-- is left out.
SELECT task_ref.task, task_ref.repo, task_ref.ref, task_ref.source, task_ref.seq
FROM task_ref
JOIN task_work ON task_work.task = task_ref.task
JOIN work_ref ON work_ref.repo = task_ref.repo
    AND work_ref.kind = task_ref.kind
    AND work_ref.ref = task_ref.ref
    AND work_ref.work = task_work.work
WHERE task_ref.kind = 'pull_request'
ORDER BY task_ref.task, task_ref.seq
