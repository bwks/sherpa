SELECT bridge, position, link
FROM manifest_bridge_link
WHERE lab = ?1
ORDER BY bridge, position
