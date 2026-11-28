SELECT node, ipv4, ipv6, mac
FROM vm_sandbox
WHERE node = ?1
