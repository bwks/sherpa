UPDATE user
SET username = ?2, password_hash = ?3, is_admin = ?4, ssh_keys = ?5, updated_at = ?6
WHERE id = ?1 AND updated_at = ?7
RETURNING id, username, password_hash, is_admin, ssh_keys, created_at, updated_at
