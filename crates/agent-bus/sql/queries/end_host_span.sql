-- The tool call ?1 returned at ?2.
UPDATE host_span SET end_ms = ?2 WHERE id = ?1 AND end_ms IS NULL
