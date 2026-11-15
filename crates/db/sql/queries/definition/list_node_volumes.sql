SELECT node, position, src, dst
FROM manifest_node_volume
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, position
