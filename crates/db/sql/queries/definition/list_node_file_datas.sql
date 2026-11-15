SELECT node, list, position, filename, content
FROM manifest_node_file_data
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, list, position
