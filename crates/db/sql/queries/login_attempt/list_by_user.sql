SELECT id, user, attempted_at, succeeded, address
FROM login_attempt
WHERE user = ?1
ORDER BY attempted_at ASC
