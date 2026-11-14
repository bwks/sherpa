-- The host's usage charged to item ?1 is charged to item ?2: ?1 was merged into it.
UPDATE host_usage SET work = ?2 WHERE work = ?1
