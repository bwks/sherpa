DELETE FROM login_limit
WHERE kind = ?1 AND username = ?2 AND address = ?3
    AND (locked_until <= ?4 OR (locked_until IS NULL AND window_start <= ?5))
