SELECT vm_sandbox.node, vm_sandbox.ipv4, vm_sandbox.ipv6, vm_sandbox.mac,
    vm_sandbox.ssh_host_key
FROM vm_sandbox
JOIN node ON node.id = vm_sandbox.node
WHERE node.lab = ?1
ORDER BY node."index"
