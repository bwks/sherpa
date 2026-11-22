INSERT INTO login_limit (kind, username, address, failures, window_start, locked_until)
VALUES (?1, ?2, ?3, 1, ?4, NULL)
