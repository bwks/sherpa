DELETE FROM login_failure
WHERE rowid = ?1 AND username = ?2 AND address = ?3 AND failed_at = ?4
