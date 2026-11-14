-- Every pull request that names an item, with the item and when it was last active: the latest
-- message of its tasks, and the latest response of the host charged to it.
SELECT work_ref.repo, work_ref.ref, work_ref.work,
    (SELECT MAX(task.last_at_ms) FROM task_work JOIN task ON task.id = task_work.task
        WHERE task_work.work = work_ref.work),
    (SELECT MAX(host_usage.at_ms) FROM host_usage WHERE host_usage.work = work_ref.work)
FROM work_ref
WHERE work_ref.kind = 'pull_request'
ORDER BY work_ref.repo, work_ref.at_ms, work_ref.ref
