DELETE FROM login_limit_pair
WHERE locked_until <= ?1 OR (locked_until IS NULL AND window_start <= ?2)
