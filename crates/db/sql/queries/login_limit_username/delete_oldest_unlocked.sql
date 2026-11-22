DELETE FROM login_limit_username
WHERE rowid = (
    SELECT rowid
    FROM login_limit_username
    WHERE locked_until IS NULL
    ORDER BY window_start
    LIMIT 1
)
