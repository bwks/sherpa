SELECT session.id, session.user, session.channel, session.created_at, session.expires_at,
    user.username, user.is_admin
FROM session
JOIN user ON user.id = session.user
WHERE session.id = ?1
