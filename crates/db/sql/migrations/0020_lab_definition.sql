-- The lab's definition: every parameter of the manifest it was deployed from, so the database,
-- not a saved manifest file, is the record of what a lab is. `up` writes it with the lab's
-- records, `redeploy` replaces the node it rebuilds, and sherpad imports it once at startup from
-- the saved manifest of a lab that has none. A value the manifest leaves out is NULL, or no row
-- in a child table; an empty list is stored as no rows.
--
-- Where the node already records what it was built with (`cpu_count`, `memory`, `ready_port`,
-- `skip_ready_check`) or refers to a record (`image`), the manifest's own value is a
-- `manifest_` column beside it.
--
-- Labs stored before this version have no definition until the import gives them one.

-- Lab-level parameters. `name` and `description` are the lab's existing columns.
ALTER TABLE lab ADD COLUMN has_definition INTEGER NOT NULL DEFAULT 0
    CHECK (has_definition IN (0, 1));
ALTER TABLE lab ADD COLUMN ready_timeout INTEGER
    CHECK (ready_timeout IS NULL OR ready_timeout >= 0);
ALTER TABLE lab ADD COLUMN provision_current_user INTEGER NOT NULL DEFAULT 0
    CHECK (provision_current_user IN (0, 1));
ALTER TABLE lab ADD COLUMN provision_concurrency INTEGER
    CHECK (provision_concurrency IS NULL OR provision_concurrency BETWEEN 1 AND 255);
-- NULL when the manifest has no `[tailscale]` table.
ALTER TABLE lab ADD COLUMN tailscale_enabled INTEGER
    CHECK (tailscale_enabled IS NULL OR tailscale_enabled IN (0, 1));
ALTER TABLE lab ADD COLUMN tailscale_auth_key_env TEXT;
-- NULL when the manifest has no `[ztp_server]` table. The password is sealed with the server's
-- database key (see `src/secret.rs`): a version byte, a 12-byte nonce, then the AES-256-GCM
-- ciphertext and its 16-byte tag.
ALTER TABLE lab ADD COLUMN ztp_server_enable INTEGER
    CHECK (ztp_server_enable IS NULL OR ztp_server_enable IN (0, 1));
ALTER TABLE lab ADD COLUMN ztp_server_username TEXT;
ALTER TABLE lab ADD COLUMN ztp_server_password BLOB
    CHECK (ztp_server_password IS NULL OR length(ztp_server_password) >= 29);

-- Node-level parameters that are one per node. The model is the node image's.
ALTER TABLE node ADD COLUMN manifest_image TEXT;
ALTER TABLE node ADD COLUMN version TEXT;
ALTER TABLE node ADD COLUMN manifest_memory INTEGER
    CHECK (manifest_memory IS NULL OR manifest_memory BETWEEN 0 AND 65535);
ALTER TABLE node ADD COLUMN manifest_cpu_count INTEGER
    CHECK (manifest_cpu_count IS NULL OR manifest_cpu_count BETWEEN 0 AND 255);
ALTER TABLE node ADD COLUMN cpu_mode TEXT
    CHECK (cpu_mode IS NULL OR cpu_mode IN ('host-model', 'host-passthrough'));
-- `cpu.topology`: all three or none.
ALTER TABLE node ADD COLUMN cpu_sockets INTEGER
    CHECK (cpu_sockets IS NULL OR cpu_sockets BETWEEN 0 AND 255);
ALTER TABLE node ADD COLUMN cpu_cores INTEGER
    CHECK (cpu_cores IS NULL OR cpu_cores BETWEEN 0 AND 255);
ALTER TABLE node ADD COLUMN cpu_threads INTEGER
    CHECK (cpu_threads IS NULL OR cpu_threads BETWEEN 0 AND 255);
ALTER TABLE node ADD COLUMN numa_node INTEGER
    CHECK (numa_node IS NULL OR numa_node BETWEEN 0 AND 255);
ALTER TABLE node ADD COLUMN numa_memory_mode TEXT
    CHECK (numa_memory_mode IS NULL OR numa_memory_mode IN ('strict', 'preferred'));
ALTER TABLE node ADD COLUMN boot_disk_size INTEGER
    CHECK (boot_disk_size IS NULL OR boot_disk_size BETWEEN 0 AND 65535);
ALTER TABLE node ADD COLUMN boot_disk_clone_method TEXT
    CHECK (boot_disk_clone_method IS NULL OR boot_disk_clone_method IN ('copy', 'overlay'));
ALTER TABLE node ADD COLUMN boot_disk_cache TEXT
    CHECK (boot_disk_cache IS NULL OR boot_disk_cache IN ('none', 'writeback', 'writethrough'));
ALTER TABLE node ADD COLUMN boot_disk_io TEXT
    CHECK (boot_disk_io IS NULL OR boot_disk_io IN ('native', 'io_uring', 'threads'));
ALTER TABLE node ADD COLUMN interfaces_data_count INTEGER
    CHECK (interfaces_data_count IS NULL OR interfaces_data_count BETWEEN 0 AND 255);
ALTER TABLE node ADD COLUMN interfaces_queues INTEGER
    CHECK (interfaces_queues IS NULL OR interfaces_queues BETWEEN 0 AND 255);
ALTER TABLE node ADD COLUMN ztp_method TEXT
    CHECK (ztp_method IS NULL OR ztp_method IN ('disabled', 'cloud-init', 'cdrom', 'disk',
        'http', 'ignition', 'nocloud', 'tftp', 'usb', 'volume'));
-- The node's startup configuration, as the client read it.
ALTER TABLE node ADD COLUMN ztp_config TEXT;
ALTER TABLE node ADD COLUMN manifest_ready_port INTEGER
    CHECK (manifest_ready_port IS NULL OR manifest_ready_port BETWEEN 0 AND 65535);
ALTER TABLE node ADD COLUMN manifest_skip_ready_check INTEGER
    CHECK (manifest_skip_ready_check IS NULL OR manifest_skip_ready_check IN (0, 1));
ALTER TABLE node ADD COLUMN vm_watchdog TEXT
    CHECK (vm_watchdog IS NULL OR vm_watchdog IN ('reset', 'poweroff', 'shutdown', 'pause',
        'dump', 'inject-nmi', 'none'));
ALTER TABLE node ADD COLUMN container_user TEXT;
ALTER TABLE node ADD COLUMN container_privileged INTEGER
    CHECK (container_privileged IS NULL OR container_privileged IN (0, 1));
ALTER TABLE node ADD COLUMN container_shm_size INTEGER;
ALTER TABLE node ADD COLUMN container_platform TEXT
    CHECK (container_platform IS NULL OR container_platform IN ('bt', 'bx'));
ALTER TABLE node ADD COLUMN unikernel_cmdline TEXT;

-- The lab's `links`, in manifest order. The `link` table is what `up` built from them.
CREATE TABLE manifest_link (
    lab TEXT NOT NULL REFERENCES lab(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    src TEXT NOT NULL,
    dst TEXT NOT NULL,
    p2p INTEGER CHECK (p2p IS NULL OR p2p IN (0, 1)),
    -- 1 when the link has an `impairment` table, whose values follow.
    impaired INTEGER NOT NULL CHECK (impaired IN (0, 1)),
    delay INTEGER CHECK (delay IS NULL OR delay BETWEEN 0 AND 4294967295),
    jitter INTEGER CHECK (jitter IS NULL OR jitter BETWEEN 0 AND 4294967295),
    loss_percent REAL,
    reorder_percent REAL,
    corrupt_percent REAL,
    PRIMARY KEY (lab, position)
) STRICT;

-- The lab's `bridges` and the `node::interface` ends each names, in manifest order.
CREATE TABLE manifest_bridge (
    lab TEXT NOT NULL REFERENCES lab(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    name TEXT NOT NULL,
    PRIMARY KEY (lab, position)
) STRICT;
CREATE TABLE manifest_bridge_link (
    lab TEXT NOT NULL,
    bridge INTEGER NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    link TEXT NOT NULL,
    PRIMARY KEY (lab, bridge, position),
    FOREIGN KEY (lab, bridge) REFERENCES manifest_bridge(lab, position) ON DELETE CASCADE
) STRICT;

-- The lab's `talos_clusters`, and each cluster's members by role, in manifest order.
CREATE TABLE manifest_talos_cluster (
    lab TEXT NOT NULL REFERENCES lab(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    name TEXT NOT NULL,
    cni TEXT NOT NULL CHECK (cni IN ('flannel', 'cilium')),
    oidc TEXT,
    vault TEXT,
    PRIMARY KEY (lab, position)
) STRICT;
CREATE TABLE manifest_talos_member (
    lab TEXT NOT NULL,
    cluster INTEGER NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('controller', 'worker')),
    position INTEGER NOT NULL CHECK (position >= 0),
    node_name TEXT NOT NULL,
    PRIMARY KEY (lab, cluster, role, position),
    FOREIGN KEY (lab, cluster) REFERENCES manifest_talos_cluster(lab, position) ON DELETE CASCADE
) STRICT;

-- A node's lists of host CPUs: `cpu.pinning`, `cpu.emulator_pinning` and
-- `boot_disk.io_thread_cpus`.
CREATE TABLE manifest_node_cpu (
    node TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    list TEXT NOT NULL CHECK (list IN ('pinning', 'emulator_pinning', 'io_thread')),
    position INTEGER NOT NULL CHECK (position >= 0),
    cpu INTEGER NOT NULL CHECK (cpu BETWEEN 0 AND 65535),
    PRIMARY KEY (node, list, position)
) STRICT;

-- A node's lists of text values, each named by `list`: `ssh.authorized_keys`, the
-- `ssh.authorized_key_files` sources and the keys the client read from them,
-- `container.commands`, `environment_variables` (as resolved by the client), the
-- `startup_scripts` and `user_scripts` paths, and the `binary_files` sources.
CREATE TABLE manifest_node_text (
    node TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    list TEXT NOT NULL CHECK (list IN ('authorized_key', 'authorized_key_file',
        'authorized_key_file_data', 'container_command', 'environment_variable',
        'startup_script', 'user_script', 'binary_file')),
    position INTEGER NOT NULL CHECK (position >= 0),
    value TEXT NOT NULL,
    PRIMARY KEY (node, list, position)
) STRICT;

-- `container.volumes`.
CREATE TABLE manifest_node_volume (
    node TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    src TEXT NOT NULL,
    dst TEXT NOT NULL,
    PRIMARY KEY (node, position)
) STRICT;

-- `text_files` as the manifest names them (`src` set), and as the client read them
-- (`content` set).
CREATE TABLE manifest_node_text_file (
    node TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    list TEXT NOT NULL CHECK (list IN ('source', 'data')),
    position INTEGER NOT NULL CHECK (position >= 0),
    src TEXT,
    content TEXT,
    dst TEXT NOT NULL,
    user TEXT NOT NULL,
    "group" TEXT NOT NULL,
    permissions INTEGER NOT NULL CHECK (permissions BETWEEN 0 AND 4294967295),
    CHECK ((list = 'source') = (src IS NOT NULL AND content IS NULL)),
    CHECK ((list = 'data') = (content IS NOT NULL AND src IS NULL)),
    PRIMARY KEY (node, list, position)
) STRICT;

-- The files the client read for `binary_files` (base64), `startup_scripts` and `user_scripts`.
CREATE TABLE manifest_node_file_data (
    node TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    list TEXT NOT NULL CHECK (list IN ('binary_file', 'startup_script', 'user_script')),
    position INTEGER NOT NULL CHECK (position >= 0),
    filename TEXT NOT NULL,
    content TEXT NOT NULL,
    PRIMARY KEY (node, list, position)
) STRICT;

-- `systemd_units` as the manifest names them (`source` set), and as the client read them
-- (`content` set, base64).
CREATE TABLE manifest_node_systemd_unit (
    node TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    list TEXT NOT NULL CHECK (list IN ('source', 'data')),
    position INTEGER NOT NULL CHECK (position >= 0),
    name TEXT NOT NULL,
    source TEXT,
    content TEXT,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    CHECK ((list = 'source') = (source IS NOT NULL AND content IS NULL)),
    CHECK ((list = 'data') = (content IS NOT NULL AND source IS NULL)),
    PRIMARY KEY (node, list, position)
) STRICT;

-- The node's `provisioners`, run by the CLI, and their files and paths. Only paths and
-- settings: no file a provisioner names is read into the database.
CREATE TABLE manifest_node_provisioner (
    node TEXT NOT NULL REFERENCES node(id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    type TEXT NOT NULL CHECK (type IN ('ssh', 'container')),
    PRIMARY KEY (node, position)
) STRICT;
-- An ssh provisioner's `files` have a `dst`; a container provisioner's have a `name`.
CREATE TABLE manifest_node_provisioner_file (
    node TEXT NOT NULL,
    provisioner INTEGER NOT NULL,
    position INTEGER NOT NULL CHECK (position >= 0),
    src TEXT NOT NULL,
    dst TEXT,
    name TEXT,
    mode INTEGER NOT NULL CHECK (mode BETWEEN 0 AND 4294967295),
    CHECK ((dst IS NULL) <> (name IS NULL)),
    PRIMARY KEY (node, provisioner, position),
    FOREIGN KEY (node, provisioner) REFERENCES manifest_node_provisioner(node, position)
        ON DELETE CASCADE
) STRICT;
-- An ssh provisioner's `env_files` and `scripts`.
CREATE TABLE manifest_node_provisioner_path (
    node TEXT NOT NULL,
    provisioner INTEGER NOT NULL,
    list TEXT NOT NULL CHECK (list IN ('env_file', 'script')),
    position INTEGER NOT NULL CHECK (position >= 0),
    path TEXT NOT NULL,
    PRIMARY KEY (node, provisioner, list, position),
    FOREIGN KEY (node, provisioner) REFERENCES manifest_node_provisioner(node, position)
        ON DELETE CASCADE
) STRICT;

-- Proof of the key that seals the database's secret values: a known value sealed with it,
-- written once with the first key. sherpad opens it at startup, so a missing or wrong key is
-- refused there, not at the first secret read.
CREATE TABLE secret_key_check (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    sealed BLOB NOT NULL CHECK (length(sealed) >= 29)
) STRICT;
