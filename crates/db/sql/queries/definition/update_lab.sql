UPDATE lab
SET has_definition = 1, ready_timeout = ?2, provision_current_user = ?3,
    provision_concurrency = ?4, tailscale_enabled = ?5, tailscale_auth_key_env = ?6,
    ztp_server_enable = ?7, ztp_server_username = ?8, ztp_server_password = ?9
WHERE id = ?1
