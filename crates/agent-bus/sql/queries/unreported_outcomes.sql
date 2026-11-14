-- The outcomes with no `ended`, oldest first: what `work backfill` parses.
SELECT seq, at_ms, task, text
FROM message
WHERE kind IN ('question', 'completed', 'failed', 'canceled') AND ended IS NULL
ORDER BY seq
