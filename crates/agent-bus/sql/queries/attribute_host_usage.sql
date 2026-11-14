-- Charge the response ?1 to the item ?2 (NULL: unattributed), unless it is pinned.
UPDATE host_usage SET work = ?2 WHERE message_id = ?1 AND pinned = 0
