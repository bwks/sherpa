-- Put task ?1 in item ?2, unless it is in one already.
INSERT INTO task_work (task, work, source, at_ms)
VALUES (?1, ?2, ?3, ?4)
ON CONFLICT (task) DO NOTHING
