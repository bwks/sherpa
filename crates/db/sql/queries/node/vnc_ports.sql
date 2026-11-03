SELECT vnc_port
FROM node
WHERE vnc_port IS NOT NULL
ORDER BY vnc_port ASC
