-- Move the refs of item ?1 to item ?2.
UPDATE work_ref SET work = ?2 WHERE work = ?1
