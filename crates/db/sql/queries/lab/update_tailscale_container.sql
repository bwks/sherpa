UPDATE lab
SET tailscale_container_id = ?2
WHERE id = ?1
RETURNING id, lab_id, name, user, loopback_network, management_network, gateway_ipv4, router_ipv4,
    tailscale_container_id, management_network_v6, gateway_ipv6, router_ipv6, loopback_network_v6,
    status, client_host, client_folder
