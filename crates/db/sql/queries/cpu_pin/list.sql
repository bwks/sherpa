SELECT lab, node_name, host_cpu, role
FROM cpu_pin
ORDER BY lab, node_name, role, host_cpu
