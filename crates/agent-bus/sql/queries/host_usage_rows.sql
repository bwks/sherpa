-- Every response of the host at or after ?1 when it is not NULL, of item ?2 when it is not
-- NULL, oldest first.
SELECT work, model, at_ms, input, output, cache_read, cache_write_5m, cache_write_1h, cost_usd
FROM host_usage
WHERE (?1 IS NULL OR at_ms >= ?1)
    AND (?2 IS NULL OR work = ?2)
ORDER BY at_ms
