SELECT role
FROM cpu_pin
WHERE host_cpu = ?3
    AND (lab <> ?1 OR node_name <> ?2)
    AND (role = 'vcpu' OR ?4 = 'vcpu')
ORDER BY role = 'vcpu' DESC
LIMIT 1
