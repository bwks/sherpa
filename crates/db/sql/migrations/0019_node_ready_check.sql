-- Each node's readiness check, which the scanner, `resume` and `redeploy` read: the port that
-- shows it is ready, and whether the check is skipped. `up` and `redeploy` write them from the
-- manifest. A node with no port is ready once it runs.
ALTER TABLE node ADD COLUMN ready_port INTEGER
    CHECK (ready_port IS NULL OR ready_port BETWEEN 1 AND 65535);
ALTER TABLE node ADD COLUMN skip_ready_check INTEGER NOT NULL DEFAULT 0
    CHECK (skip_ready_check IN (0, 1));

-- Nodes stored before this version get the check the scanner ran on them: a virtual machine
-- its model's port, which is SSH but for Talos Linux's API port, and a container or unikernel
-- none. None is skipped, as the scanner never knew of a skip. Released migrations never change,
-- so the kind, model and port numbers are written out.
UPDATE node
SET ready_port = CASE
    WHEN (SELECT model FROM node_image WHERE node_image.id = node.image) = 'talos_linux'
        THEN 50000
    ELSE 22
END
WHERE (SELECT kind FROM node_image WHERE node_image.id = node.image) = 'virtual_machine';
