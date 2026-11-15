SELECT position, name
FROM manifest_bridge
WHERE lab = ?1
ORDER BY position
