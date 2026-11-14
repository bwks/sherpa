-- The responses with no cost.
SELECT message_id, model, speed, input, output, cache_read, cache_write_5m, cache_write_1h
FROM host_usage
WHERE cost_usd IS NULL
