INSERT INTO lab (id, lab_id, name, user, loopback_network, management_network, gateway_ipv4,
    router_ipv4, tailscale_container_id, management_network_v6, gateway_ipv6, router_ipv6,
    loopback_network_v6, status, client_host, client_folder, description)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
RETURNING id, lab_id, name, user, loopback_network, management_network, gateway_ipv4, router_ipv4,
    tailscale_container_id, management_network_v6, gateway_ipv6, router_ipv6, loopback_network_v6,
    status, client_host, client_folder, description
