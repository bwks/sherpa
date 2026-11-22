SELECT failures, locked_until
FROM login_limit_pair
WHERE username = ?1 AND address = ?2
