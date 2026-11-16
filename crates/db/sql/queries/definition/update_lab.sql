UPDATE lab
SET has_definition = 1, ready_timeout = ?2, provision_concurrency = ?3, tailscale_enabled = ?4,
    tailscale_auth_key_env = ?5, ztp_server_enable = ?6, ztp_server_username = ?7,
    ztp_server_password = ?8
WHERE id = ?1
