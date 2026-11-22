DELETE FROM login_limit_pair
WHERE username = ?1 AND address = ?2
    AND (locked_until <= ?3 OR (locked_until IS NULL AND window_start <= ?4))
