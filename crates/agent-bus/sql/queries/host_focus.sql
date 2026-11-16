-- Every change of the host's active item, oldest first.
SELECT from_ms, session, work, source, task, pr_repo, pr_number
FROM host_focus
ORDER BY from_ms, id
