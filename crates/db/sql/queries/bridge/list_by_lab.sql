SELECT id, "index", bridge_name, network_name, lab
FROM bridge
WHERE lab = ?1
ORDER BY "index"
