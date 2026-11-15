SELECT id, ztp_password
FROM node_image
WHERE ztp_password IS NOT NULL AND ztp_password NOT GLOB ?1
ORDER BY id
