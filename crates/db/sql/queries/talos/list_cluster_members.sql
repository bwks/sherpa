SELECT manifest_talos_member.cluster, manifest_talos_member.role,
    manifest_talos_member.node_name, node.mgmt_ipv4
FROM manifest_talos_member
LEFT JOIN node ON node.lab = manifest_talos_member.lab
    AND node.name = manifest_talos_member.node_name
WHERE manifest_talos_member.lab = ?1
ORDER BY manifest_talos_member.cluster, manifest_talos_member.role,
    manifest_talos_member.position
