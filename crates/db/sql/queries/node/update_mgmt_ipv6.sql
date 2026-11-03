UPDATE node
SET mgmt_ipv6 = ?2
WHERE id = ?1
RETURNING id, name, "index", image, lab, mgmt_ipv4, mgmt_ipv6, mgmt_mac, state, cpu_count, memory,
    vnc_port, ssh_host_key, cpu_pinning, qemu_thread_cpus
