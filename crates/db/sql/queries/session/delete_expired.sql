DELETE FROM session
WHERE expires_at <= ?1
