-- The refs of item ?1, all items when ?1 is NULL.
SELECT work, repo, kind, ref, source, at_ms
FROM work_ref
WHERE ?1 IS NULL OR work = ?1
ORDER BY work, kind, ref
