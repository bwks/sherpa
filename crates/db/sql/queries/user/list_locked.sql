SELECT username
FROM user
WHERE locked_at IS NOT NULL
ORDER BY username ASC
