UPDATE login_limit_pair
SET failures = failures - 1
WHERE username = ?1 AND address = ?2 AND failures > 0
