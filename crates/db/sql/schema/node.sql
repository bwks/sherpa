
CREATE TABLE node (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    "index" INTEGER NOT NULL CHECK ("index" BETWEEN 0 AND 65535),
    image TEXT NOT NULL REFERENCES node_image(id) ON DELETE RESTRICT,
    lab TEXT NOT NULL REFERENCES lab(id) ON DELETE CASCADE,
    mgmt_ipv4 TEXT,
    mgmt_ipv6 TEXT,
    mgmt_mac TEXT,
    state TEXT NOT NULL,
    cpu_count INTEGER CHECK (cpu_count IS NULL OR cpu_count BETWEEN 1 AND 255),
    memory INTEGER CHECK (memory IS NULL OR memory BETWEEN 1 AND 65535),
    vnc_port INTEGER UNIQUE CHECK (vnc_port IS NULL OR vnc_port BETWEEN 1 AND 65535),
    ssh_host_key TEXT,
    cpu_pinning TEXT CHECK (cpu_pinning IS NULL OR json_valid(cpu_pinning)),
    qemu_thread_cpus TEXT CHECK (qemu_thread_cpus IS NULL OR json_valid(qemu_thread_cpus)),
    UNIQUE (lab, name),
    UNIQUE (lab, "index")
) STRICT;
CREATE INDEX node_image_ref ON node (image);
