-- Move the tasks of item ?1 to item ?2.
UPDATE task_work SET work = ?2 WHERE work = ?1
