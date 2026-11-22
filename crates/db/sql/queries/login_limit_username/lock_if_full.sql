UPDATE login_limit_username
SET locked_until = ?3
WHERE username = ?1 AND locked_until IS NULL AND failures >= ?2
RETURNING failures
