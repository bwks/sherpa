-- Set the import fact ?1 to ?2, unless it is set.
INSERT INTO host_meta (key, value) VALUES (?1, ?2)
ON CONFLICT (key) DO NOTHING
