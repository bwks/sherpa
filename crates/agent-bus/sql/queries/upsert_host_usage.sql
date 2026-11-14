-- Record the response ?1, or replace its figures with the merged ones. Its item and pin stay.
INSERT INTO host_usage (message_id, session, agent_id, at_ms, last_at_ms, model, speed, input,
    output, cache_read, cache_write_5m, cache_write_1h, cost_usd, work, pinned)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL, 0)
ON CONFLICT (message_id) DO UPDATE SET
    session = excluded.session,
    agent_id = excluded.agent_id,
    at_ms = excluded.at_ms,
    last_at_ms = excluded.last_at_ms,
    model = excluded.model,
    speed = excluded.speed,
    input = excluded.input,
    output = excluded.output,
    cache_read = excluded.cache_read,
    cache_write_5m = excluded.cache_write_5m,
    cache_write_1h = excluded.cache_write_1h,
    cost_usd = excluded.cost_usd
