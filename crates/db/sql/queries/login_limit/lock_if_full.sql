UPDATE login_limit
SET locked_until = ?5
WHERE kind = ?1 AND username = ?2 AND address = ?3 AND locked_until IS NULL AND failures >= ?4
RETURNING kind
