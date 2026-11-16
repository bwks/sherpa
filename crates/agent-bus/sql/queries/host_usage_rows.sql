-- Every response of the host at or after ?1 when it is not NULL, of item ?2 when it is not
-- NULL, oldest first, with its session, the time it is charged at (a subagent's start, else
-- its own) and the pull request it is pinned to.
SELECT work, model, at_ms, input, output, cache_read, cache_write_5m, cache_write_1h, cost_usd,
    session,
    COALESCE(
        (SELECT MIN(host_transcript.started_at_ms) FROM host_transcript
            WHERE host_transcript.agent_id = host_usage.agent_id),
        at_ms
    ),
    pr_repo, pr_number
FROM host_usage
WHERE (?1 IS NULL OR at_ms >= ?1)
    AND (?2 IS NULL OR work = ?2)
ORDER BY at_ms
