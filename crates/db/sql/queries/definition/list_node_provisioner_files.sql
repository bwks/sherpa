SELECT node, provisioner, position, src, dst, name, mode
FROM manifest_node_provisioner_file
WHERE node IN (SELECT id FROM node WHERE lab = ?1)
ORDER BY node, provisioner, position
