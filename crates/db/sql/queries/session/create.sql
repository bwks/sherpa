INSERT INTO session (id, user, channel, created_at, expires_at)
VALUES (?1, ?2, ?3, ?4, ?5)
RETURNING id, user, channel, created_at, expires_at
