UPDATE user
SET failed_logins = failed_logins + 1,
    locked_at = CASE WHEN failed_logins + 1 >= ?2 THEN ?3 ELSE NULL END
WHERE username = ?1 AND locked_at IS NULL
RETURNING id, username, password_hash, is_admin, ssh_keys, created_at, updated_at, locked_at
