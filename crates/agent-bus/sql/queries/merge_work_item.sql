-- Mark item ?1 merged into item ?2.
UPDATE work_item SET merged_into = ?2 WHERE id = ?1
