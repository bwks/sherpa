SELECT node, list, position, value
FROM manifest_node_text
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, list, position
