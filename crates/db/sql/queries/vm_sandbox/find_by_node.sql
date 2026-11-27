SELECT node, ipv4, ipv6, mac, ssh_host_key
FROM vm_sandbox
WHERE node = ?1
