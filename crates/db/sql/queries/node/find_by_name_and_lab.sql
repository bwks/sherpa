SELECT id, name, "index", image, lab, mgmt_ipv4, mgmt_ipv6, mgmt_mac, state, cpu_count, memory,
    ssh_host_key, ready_port, skip_ready_check, ready_sshd_marker,
    ssh_username
FROM node
WHERE name = ?1 AND lab = ?2
