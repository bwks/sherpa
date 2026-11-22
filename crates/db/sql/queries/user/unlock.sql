UPDATE user
SET failed_logins = 0, locked_at = NULL
WHERE username = ?1
RETURNING id
