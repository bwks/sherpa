UPDATE lab
SET ready_timeout = ?2, provision_current_user = ?3, provision_concurrency = ?4
WHERE id = ?1
