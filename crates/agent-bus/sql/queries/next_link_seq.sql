-- The `seq` of a ref task ?1 is linked to by hand: one past the last message of the record and
-- every ref of the task, so the last `link` is the one named last.
SELECT max(
    coalesce((SELECT max(seq) FROM message), 0),
    coalesce((SELECT max(seq) FROM task_ref WHERE task = ?1), 0)
) + 1
