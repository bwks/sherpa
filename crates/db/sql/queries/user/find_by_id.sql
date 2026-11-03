SELECT id, username, password_hash, is_admin, ssh_keys, created_at, updated_at
FROM user
WHERE id = ?1
