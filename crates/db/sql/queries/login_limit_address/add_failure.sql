UPDATE login_limit_address
SET failures = failures + 1
WHERE address = ?1
