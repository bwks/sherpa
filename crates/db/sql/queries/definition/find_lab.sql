SELECT name, description, has_definition, ready_timeout, provision_concurrency,
    tailscale_enabled, tailscale_auth_key_env
FROM lab
WHERE id = ?1
