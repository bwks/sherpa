DELETE FROM login_limit
WHERE rowid = (
    SELECT rowid
    FROM login_limit
    WHERE kind = ?1 AND locked_until IS NULL
    ORDER BY window_start
    LIMIT 1
)
