INSERT INTO cpu_pin (lab, node_name, host_cpu, role)
SELECT ?1, ?2, ?3, ?4
WHERE NOT EXISTS (
    SELECT 1 FROM cpu_pin WHERE lab = ?1 AND node_name = ?2 AND host_cpu = ?3 AND role = ?4
)
