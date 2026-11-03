UPDATE node
SET name = ?2, "index" = ?3, image = ?4, lab = ?5, mgmt_ipv4 = ?6, mgmt_ipv6 = ?7, mgmt_mac = ?8,
    state = ?9, cpu_count = ?10, memory = ?11, vnc_port = ?12, ssh_host_key = ?13,
    cpu_pinning = ?14, qemu_thread_cpus = ?15
WHERE id = ?1
RETURNING id, name, "index", image, lab, mgmt_ipv4, mgmt_ipv6, mgmt_mac, state, cpu_count, memory,
    vnc_port, ssh_host_key, cpu_pinning, qemu_thread_cpus
