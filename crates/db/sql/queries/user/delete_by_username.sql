DELETE FROM user
WHERE username = ?1
RETURNING id, username, password_hash, is_admin, ssh_keys, created_at, updated_at
