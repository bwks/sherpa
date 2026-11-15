UPDATE lab
SET ready_timeout = ?2, provision_current_user = ?3
WHERE id = ?1
