DELETE FROM login_limit_username
WHERE username = ?1 AND failures = 0 AND locked_until IS NULL
