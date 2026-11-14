-- The response ?1, as recorded.
SELECT session, agent_id, at_ms, last_at_ms, model, speed, input, output, cache_read,
    cache_write_5m, cache_write_1h, cost_usd
FROM host_usage
WHERE message_id = ?1
