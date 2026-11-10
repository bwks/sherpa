-- Every model with images has a default. A model left without one by an earlier release (which
-- let an import, download or delete drop it) gets its most recently added image as the default,
-- as deleting the default now does.
UPDATE node_image SET "default" = 1
WHERE rowid IN (
    SELECT max(rowid) FROM node_image GROUP BY model, kind HAVING max("default") = 0
);
