DELETE FROM login_limit_pair
WHERE rowid = (
    SELECT rowid
    FROM login_limit_pair
    WHERE locked_until IS NULL
    ORDER BY window_start
    LIMIT 1
)
