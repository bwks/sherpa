SELECT failures, locked_until
FROM login_limit_address
WHERE address = ?1
