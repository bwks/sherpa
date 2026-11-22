DELETE FROM login_limit_address
WHERE address = ?1 AND failures = 0 AND locked_until IS NULL
