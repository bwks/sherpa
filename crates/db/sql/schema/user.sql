
CREATE TABLE user (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE
        CHECK (length(username) BETWEEN 3 AND 32
            AND substr(username, 1, 1) GLOB '[a-z_]'
            AND username NOT GLOB '*[^a-z0-9_-]*'),
    password_hash TEXT NOT NULL,
    is_admin INTEGER NOT NULL CHECK (is_admin IN (0, 1)),
    ssh_keys TEXT NOT NULL CHECK (json_valid(ssh_keys)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
) STRICT;
