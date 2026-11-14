-- The host's focus on item ?1 is on item ?2: ?1 was merged into it.
UPDATE host_focus SET work = ?2 WHERE work = ?1
