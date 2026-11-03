SELECT id, "index", bridge_name, network_name, lab
FROM bridge
WHERE "index" = ?1 AND lab = ?2
