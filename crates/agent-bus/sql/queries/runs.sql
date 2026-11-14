-- Every run, with each of its models (none for a run whose usage named none), in order.
SELECT run.seq, run.task, run.model, run.input, run.output, run.cache_creation,
    run.cache_read, run.cost_usd, run_model.model, run_model.input, run_model.output,
    run_model.cache_creation, run_model.cache_read, run_model.cost_usd
FROM run
LEFT JOIN run_model ON run_model.run = run.seq
ORDER BY run.seq, run_model.model
