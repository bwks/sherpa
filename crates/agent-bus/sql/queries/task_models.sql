-- Every model the runs of each task of item ?1 used.
SELECT DISTINCT run.task, run_model.model
FROM run
JOIN run_model ON run_model.run = run.seq
JOIN task_work ON task_work.task = run.task
WHERE task_work.work = ?1
ORDER BY run.task, run_model.model
