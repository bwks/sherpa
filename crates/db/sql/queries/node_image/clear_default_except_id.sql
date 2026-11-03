UPDATE node_image
SET "default" = 0
WHERE model = ?1 AND kind = ?2 AND id != ?3
