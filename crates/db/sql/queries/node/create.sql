INSERT INTO node (id, name, "index", image, lab, mgmt_ipv4, mgmt_ipv6, mgmt_mac, state, cpu_count,
    memory, vnc_port, ssh_host_key, cpu_pinning, qemu_thread_cpus, ready_port, skip_ready_check,
    ssh_username)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
RETURNING id, name, "index", image, lab, mgmt_ipv4, mgmt_ipv6, mgmt_mac, state, cpu_count, memory,
    vnc_port, ssh_host_key, cpu_pinning, qemu_thread_cpus, ready_port, skip_ready_check,
    ssh_username
