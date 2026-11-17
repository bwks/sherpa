-- Whether a node's SSH server must show Sherpa's ready marker in its identification line before
-- the node counts as ready: a Windows node with cloud-init ZTP, whose SSH server answers before
-- its owner can log in. `up` and `redeploy` set it with the rest of the node's readiness check,
-- so the scanner, `resume` and `node.host_keys` decide readiness as they do. Nodes already built
-- keep 0: a Windows node from before Sherpa's sshd config shows no marker, and would never be
-- ready on it.
ALTER TABLE node ADD COLUMN ready_sshd_marker INTEGER NOT NULL DEFAULT 0
    CHECK (ready_sshd_marker IN (0, 1));
