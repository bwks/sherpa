UPDATE lab
SET phase = ?3
WHERE lab_id = ?1 AND phase = ?2
RETURNING id, lab_id, name, user, loopback_network, management_network, gateway_ipv4, router_ipv4,
    tailscale_container_id, management_network_v6, gateway_ipv6, router_ipv6, loopback_network_v6,
    client_host, client_folder, description, phase
