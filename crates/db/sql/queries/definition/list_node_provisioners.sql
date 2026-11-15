SELECT node, position, type
FROM manifest_node_provisioner
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, position
