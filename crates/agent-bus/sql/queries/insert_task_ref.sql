-- Keep that task ?1 named the ref (?2, ?3, ?4), from source ?5 in message ?6, the first time it
-- does. A `manual` one (`link`) takes over the row, whatever named it before, at its `seq`.
INSERT INTO task_ref (task, repo, kind, ref, source, seq)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT (task, repo, kind, ref) DO UPDATE
SET source = excluded.source, seq = excluded.seq
WHERE excluded.source = 'manual'
