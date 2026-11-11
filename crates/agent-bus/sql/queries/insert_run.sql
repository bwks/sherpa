INSERT INTO run (seq, task, outcome, model, input, output, cache_creation, cache_read, num_turns,
    duration_ms, duration_api_ms, cost_usd, session_cost_usd)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
ON CONFLICT (seq) DO NOTHING
