-- The items merged into item ?1.
SELECT id, created_at_ms FROM work_item WHERE merged_into = ?1 ORDER BY id
