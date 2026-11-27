-- The record of a node's sandbox: the management address the sandbox has of its own, beside its
-- node's, and the MAC of the VM's NIC in a network namespace its network OS does not see, named
-- `<node>-sbx` on the lab's boot server. `up` records it with the node, `down`, `resume` and
-- `redeploy` keep it, and it goes when the node or its lab is deleted.
CREATE TABLE sandbox_node (
    node TEXT PRIMARY KEY REFERENCES node(id) ON DELETE CASCADE,
    ipv4 TEXT NOT NULL,
    ipv6 TEXT,
    mac TEXT NOT NULL
) STRICT;
