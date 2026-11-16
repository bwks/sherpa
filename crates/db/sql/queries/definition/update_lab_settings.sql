UPDATE lab
SET ready_timeout = ?2, provision_concurrency = ?3
WHERE id = ?1
