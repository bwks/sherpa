SELECT lab, node_name, host_cpu, role
FROM cpu_pin
WHERE lab = ?1 AND node_name = ?2
ORDER BY role, host_cpu
