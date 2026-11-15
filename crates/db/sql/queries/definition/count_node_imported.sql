SELECT count(*)
FROM node
WHERE lab = ?1 AND name = ?2 AND definition_imported = 1
