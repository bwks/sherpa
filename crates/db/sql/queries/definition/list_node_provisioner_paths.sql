SELECT node, provisioner, list, position, path
FROM manifest_node_provisioner_path
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, provisioner, list, position
