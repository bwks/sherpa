SELECT node, ipv4, ipv6, mac
FROM sandbox_address
WHERE node = ?1
