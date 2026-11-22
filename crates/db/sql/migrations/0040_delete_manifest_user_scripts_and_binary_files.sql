-- The manifest's `user_scripts` and `binary_files` are removed (#781): an ssh provisioner's
-- `scripts` and `files` do their job, and the definition no longer reads these lists, so the
-- rows stored for them are deleted and older labs' definitions still load. The `list` CHECK
-- keeps the two names, as `manifest_node_text` cannot be rebuilt in place; nothing writes them
-- again.
DELETE FROM manifest_node_text WHERE list IN ('user_script', 'binary_file');
