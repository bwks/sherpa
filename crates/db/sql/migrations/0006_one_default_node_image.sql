-- One default image per model and kind. Defaults left twice by an earlier create, which did not
-- clear the others, keep the most recently written image as the default; the index then refuses a
-- second default.
UPDATE node_image SET "default" = 0
WHERE "default" = 1
    AND rowid NOT IN (SELECT max(rowid) FROM node_image WHERE "default" = 1 GROUP BY model, kind);
CREATE UNIQUE INDEX node_image_one_default ON node_image (model, kind) WHERE "default" = 1;
