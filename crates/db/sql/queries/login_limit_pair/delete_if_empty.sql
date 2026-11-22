DELETE FROM login_limit_pair
WHERE username = ?1 AND address = ?2 AND failures = 0 AND locked_until IS NULL
