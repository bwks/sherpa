-- Every item not merged into another, oldest first.
SELECT id, created_at_ms FROM work_item WHERE merged_into IS NULL ORDER BY id
