UPDATE lab
SET lab_id = ?2, name = ?3, user = ?4, loopback_network = ?5, management_network = ?6,
    gateway_ipv4 = ?7, router_ipv4 = ?8, tailscale_container_id = ?9, management_network_v6 = ?10,
    gateway_ipv6 = ?11, router_ipv6 = ?12, loopback_network_v6 = ?13, status = ?14
WHERE id = ?1
RETURNING id, lab_id, name, user, loopback_network, management_network, gateway_ipv4, router_ipv4,
    tailscale_container_id, management_network_v6, gateway_ipv6, router_ipv6, loopback_network_v6,
    status
