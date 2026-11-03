
CREATE TABLE lab (
    id TEXT PRIMARY KEY,
    lab_id TEXT NOT NULL UNIQUE CHECK (length(lab_id) >= 1),
    name TEXT NOT NULL,
    user TEXT NOT NULL REFERENCES user(id) ON DELETE RESTRICT,
    loopback_network TEXT NOT NULL UNIQUE,
    management_network TEXT NOT NULL UNIQUE,
    gateway_ipv4 TEXT NOT NULL,
    router_ipv4 TEXT NOT NULL,
    tailscale_container_id TEXT,
    management_network_v6 TEXT UNIQUE,
    gateway_ipv6 TEXT,
    router_ipv6 TEXT,
    loopback_network_v6 TEXT UNIQUE,
    status TEXT NOT NULL,
    UNIQUE (name, user)
) STRICT;
CREATE INDEX lab_user ON lab (user);
