INSERT INTO bridge (id, "index", bridge_name, network_name, lab)
VALUES (?1, ?2, ?3, ?4, ?5)
RETURNING id, "index", bridge_name, network_name, lab
