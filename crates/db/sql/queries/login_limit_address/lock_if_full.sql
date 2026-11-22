UPDATE login_limit_address
SET locked_until = ?3
WHERE address = ?1 AND locked_until IS NULL AND failures >= ?2
RETURNING failures
