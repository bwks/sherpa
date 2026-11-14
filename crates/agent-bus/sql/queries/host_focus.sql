-- Every change of the host's active item, oldest first.
SELECT from_ms, session, work, source FROM host_focus ORDER BY from_ms, id
