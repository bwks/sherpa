UPDATE login_limit
SET failures = failures - 1
WHERE kind = ?1 AND username = ?2 AND address = ?3 AND failures > 0
