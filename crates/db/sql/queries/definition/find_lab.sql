SELECT id, name, description, has_definition, ready_timeout, provision_concurrency,
    tailscale_enabled, tailscale_auth_key_env, ztp_server_enable,
    ztp_server_username, ztp_server_password
FROM lab
WHERE id = ?1
