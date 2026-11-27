SELECT sandbox_node.node, sandbox_node.ipv4, sandbox_node.ipv6, sandbox_node.mac
FROM sandbox_node
JOIN node ON node.id = sandbox_node.node
WHERE node.lab = ?1
ORDER BY node."index"
