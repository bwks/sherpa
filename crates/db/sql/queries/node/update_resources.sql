UPDATE node
SET cpu_count = ?2, memory = ?3, vnc_port = ?4, cpu_pinning = ?5, qemu_thread_cpus = ?6
WHERE id = ?1
RETURNING id, name, "index", image, lab, mgmt_ipv4, mgmt_ipv6, mgmt_mac, state, cpu_count, memory,
    vnc_port, ssh_host_key, cpu_pinning, qemu_thread_cpus
