SELECT sandbox_address.node, sandbox_address.ipv4, sandbox_address.ipv6, sandbox_address.mac
FROM sandbox_address
JOIN node ON node.id = sandbox_address.node
WHERE node.lab = ?1
ORDER BY node."index"
