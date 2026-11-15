-- The host CPUs each node holds, one row per CPU and role: `vcpu` for the node's vCPUs, `thread`
-- for its QEMU emulator and disk I/O threads. Admission claims them when it admits a node, so a
-- lab's rows exist from `up`'s validation on, before its node rows. A node is its lab and its
-- name in the lab. The rows are held while the lab is stopped, and go when the lab or the node
-- is deleted. They replace `node.cpu_pinning` and `node.qemu_thread_cpus`.
CREATE TABLE cpu_pin (
    lab TEXT NOT NULL REFERENCES lab(id) ON DELETE CASCADE,
    node_name TEXT NOT NULL,
    host_cpu INTEGER NOT NULL CHECK (host_cpu BETWEEN 0 AND 65535),
    role TEXT NOT NULL CHECK (role IN ('vcpu', 'thread')),
    PRIMARY KEY (lab, node_name, host_cpu, role)
) STRICT;
CREATE INDEX cpu_pin_host_cpu ON cpu_pin (host_cpu);

-- Every node's stored pins, as they are. Labs stored before this version may already share a
-- CPU against the rules below; they are copied before the trigger exists, so it does not refuse
-- them, and sherpad warns of each such overlap while both nodes hold it.
INSERT INTO cpu_pin (lab, node_name, host_cpu, role)
    SELECT DISTINCT node.lab, node.name, cpu.value, 'vcpu'
    FROM node, json_each(node.cpu_pinning) AS cpu
    WHERE node.cpu_pinning IS NOT NULL;
INSERT INTO cpu_pin (lab, node_name, host_cpu, role)
    SELECT DISTINCT node.lab, node.name, cpu.value, 'thread'
    FROM node, json_each(node.qemu_thread_cpus) AS cpu
    WHERE node.qemu_thread_cpus IS NOT NULL;

-- A host CPU pinned to a node's vCPUs belongs to that node alone; QEMU threads of different
-- nodes may share a CPU. A new row is refused when another node holds its CPU and either row is
-- a vCPU. A node's own rows never conflict: while a redeploy runs, the node holds its old and
-- its new pins together, and its vCPUs and threads are kept apart by the manifest checks. Rows
-- are claimed and given back, never changed. Only new rows are checked, so rows copied above
-- that already share a CPU stay.
CREATE TRIGGER cpu_pin_held BEFORE INSERT ON cpu_pin
    WHEN EXISTS (
        SELECT 1 FROM cpu_pin
        WHERE host_cpu = NEW.host_cpu
            AND (lab <> NEW.lab OR node_name <> NEW.node_name)
            AND (role = 'vcpu' OR NEW.role = 'vcpu')
    )
    BEGIN SELECT RAISE(ABORT, 'Host CPU is already pinned by another node'); END;
CREATE TRIGGER cpu_pin_unchanged BEFORE UPDATE ON cpu_pin
    BEGIN SELECT RAISE(ABORT, 'A CPU pin is claimed or given back, never changed'); END;
CREATE TRIGGER cpu_pin_node_deleted AFTER DELETE ON node
    BEGIN DELETE FROM cpu_pin WHERE lab = OLD.lab AND node_name = OLD.name; END;

ALTER TABLE node DROP COLUMN cpu_pinning;
ALTER TABLE node DROP COLUMN qemu_thread_cpus;
