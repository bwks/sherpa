-- Move to item ?1, and pin, the responses charged to item ?2 (unattributed ones when ?3), at or
-- after ?4 and before ?5 when they are not NULL, of the session ?6 when it is not NULL.
UPDATE host_usage SET work = ?1, pinned = 1
WHERE ((?3 AND work IS NULL) OR (NOT ?3 AND work = ?2))
    AND (?4 IS NULL OR at_ms >= ?4)
    AND (?5 IS NULL OR at_ms < ?5)
    AND (?6 IS NULL OR session = ?6)
