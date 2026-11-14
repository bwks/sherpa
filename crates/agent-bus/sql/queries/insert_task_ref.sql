-- Keep that task ?1 named the ref (?2, ?3, ?4), the first time it does.
INSERT INTO task_ref (task, repo, kind, ref, source, seq)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT (task, repo, kind, ref) DO NOTHING
