SELECT failures, locked_until
FROM login_limit
WHERE kind = ?1 AND username = ?2 AND address = ?3
