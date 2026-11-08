-- The key pair the `sherpa` account on the lab's nodes logs in with: `lab`, the lab's own,
-- made by `up` in the lab folder, or `server`, the server's shared key, which labs created
-- before this version keep.
ALTER TABLE lab ADD COLUMN ssh_key TEXT NOT NULL DEFAULT 'server';
