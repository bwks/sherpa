SELECT device_password IS NOT NULL AS device_password_set
FROM user
WHERE id = ?1
