SELECT node, list, position, name, source, content, enabled
FROM manifest_node_systemd_unit
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, list, position
