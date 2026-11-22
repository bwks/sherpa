INSERT INTO login_failure (username, address, failed_at)
VALUES (?1, ?2, ?3)
RETURNING rowid
