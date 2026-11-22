SELECT failures, locked_until
FROM login_limit_username
WHERE username = ?1
