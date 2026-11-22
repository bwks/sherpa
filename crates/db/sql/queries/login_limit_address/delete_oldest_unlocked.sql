DELETE FROM login_limit_address
WHERE rowid = (
    SELECT rowid
    FROM login_limit_address
    WHERE locked_until IS NULL
    ORDER BY window_start
    LIMIT 1
)
