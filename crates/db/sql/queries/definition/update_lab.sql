UPDATE lab
SET has_definition = 1, ready_timeout = ?2, provision_concurrency = ?3, tailscale_enabled = ?4,
    tailscale_auth_key_env = ?5
WHERE id = ?1
