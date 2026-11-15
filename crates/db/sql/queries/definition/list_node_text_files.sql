SELECT node, list, position, src, content, dst, user, "group", permissions
FROM manifest_node_text_file
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, list, position
