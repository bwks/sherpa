INSERT INTO user (id, username, password_hash, is_admin, ssh_keys, created_at, updated_at)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
RETURNING id, username, password_hash, is_admin, ssh_keys, created_at, updated_at
