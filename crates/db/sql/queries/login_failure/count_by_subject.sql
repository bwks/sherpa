SELECT
    (SELECT count(*) FROM login_failure
        WHERE username = ?1 AND address = ?2 AND failed_at > ?3) AS pair,
    (SELECT count(*) FROM login_failure
        WHERE address = ?2 AND failed_at > ?3) AS address,
    (SELECT count(*) FROM login_failure
        WHERE username = ?1 AND failed_at > ?3) AS username
