-- The account a node's SSH config logs in with: the lab owner's login on a node that provisions
-- it, else NULL, for the shared `sherpa` account and the lab's key. `up` and `redeploy` write
-- it. It was held only in the lab's `lab-info.toml` on the server before this version; sherpad
-- copies it from there at startup and deletes that file, with the lab's SSH config and
-- `known_hosts`, which are built from the database when a client asks for them.
ALTER TABLE node ADD COLUMN ssh_username TEXT
    CHECK (ssh_username IS NULL OR length(ssh_username) > 0);
