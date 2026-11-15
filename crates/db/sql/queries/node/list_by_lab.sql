SELECT id, name, "index", image, lab, mgmt_ipv4, mgmt_ipv6, mgmt_mac, state, cpu_count, memory,
    vnc_port, ssh_host_key, ready_port, skip_ready_check, ssh_username
FROM node
WHERE lab = ?1
ORDER BY name ASC
