SELECT count(*)
FROM link
WHERE node_a = ?1 OR node_b = ?1
