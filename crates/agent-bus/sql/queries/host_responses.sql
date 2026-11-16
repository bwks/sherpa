-- The responses of the host's main conversations (not subagents), with their item, model and
-- the pull request they are pinned to.
SELECT session, at_ms, last_at_ms, work, model, pr_repo, pr_number
FROM host_usage
WHERE agent_id IS NULL
ORDER BY session, at_ms
