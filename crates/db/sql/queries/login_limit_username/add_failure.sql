UPDATE login_limit_username
SET failures = failures + 1
WHERE username = ?1
