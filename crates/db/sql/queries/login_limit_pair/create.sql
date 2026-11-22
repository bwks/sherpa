INSERT INTO login_limit_pair (username, address, failures, window_start, locked_until)
VALUES (?1, ?2, 1, ?3, NULL)
