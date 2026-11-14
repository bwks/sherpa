-- Name item ?4 by the ref (?1, ?2, ?3), unless the ref already names an item.
INSERT INTO work_ref (repo, kind, ref, work, source, at_ms)
VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT (repo, kind, ref) DO NOTHING
