-- Create the vm_sandbox table: each sandboxed node's sandbox VM address and MAC.
CREATE TABLE vm_sandbox (
    node TEXT PRIMARY KEY REFERENCES node(id) ON DELETE CASCADE,
    ipv4 TEXT NOT NULL,
    ipv6 TEXT,
    mac TEXT NOT NULL
) STRICT;
