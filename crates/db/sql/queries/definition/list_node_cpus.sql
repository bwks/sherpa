SELECT node, list, position, cpu
FROM manifest_node_cpu
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, list, position
