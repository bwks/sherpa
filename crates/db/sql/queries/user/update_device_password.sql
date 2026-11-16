UPDATE user
SET device_password = ?2, updated_at = ?3
WHERE id = ?1
RETURNING id
