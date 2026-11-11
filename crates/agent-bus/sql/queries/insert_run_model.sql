INSERT INTO run_model (run, model, canonical_model, cost_basis, input, output, cache_creation,
    cache_read, cost_usd)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
ON CONFLICT (run, model) DO NOTHING
