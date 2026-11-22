UPDATE user
SET failed_logins = 0, locked_at = NULL
WHERE id = ?1 AND (locked_at IS NULL OR locked_at = ?2)
RETURNING id
