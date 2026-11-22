DELETE FROM login_limit
WHERE kind = ?1 AND (locked_until <= ?2 OR (locked_until IS NULL AND window_start <= ?3))
