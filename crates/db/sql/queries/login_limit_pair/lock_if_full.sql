UPDATE login_limit_pair
SET locked_until = ?4
WHERE username = ?1 AND address = ?2 AND locked_until IS NULL AND failures >= ?3
RETURNING failures
