DELETE FROM login_limit
WHERE kind = ?1 AND username = ?2 AND address = ?3 AND failures = 0 AND locked_until IS NULL
