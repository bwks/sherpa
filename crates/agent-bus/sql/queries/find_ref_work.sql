-- The item the ref (?1 repo, ?2 kind, ?3 ref) names, if any.
SELECT work FROM work_ref WHERE repo = ?1 AND kind = ?2 AND ref = ?3
