SELECT cluster, role, position, node_name
FROM manifest_talos_member
WHERE lab = ?1
ORDER BY cluster, role, position
