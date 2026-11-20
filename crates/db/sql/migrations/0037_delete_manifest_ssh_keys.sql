-- The manifest's per-node `ssh` group (`authorized_keys`, `authorized_key_files`) is removed
-- (#754): every node takes the keys on its owner's account, and the definition no longer reads
-- these lists, so the rows stored for them are deleted and older labs' definitions still load.
-- The `list` CHECK keeps the two names, as `manifest_node_text` cannot be rebuilt in place;
-- nothing writes them again.
DELETE FROM manifest_node_text WHERE list IN ('authorized_key', 'authorized_key_file');
