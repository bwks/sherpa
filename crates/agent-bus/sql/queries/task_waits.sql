-- Every question and answer, by task, oldest first.
SELECT task, kind, at_ms FROM message WHERE kind IN ('question', 'answer') ORDER BY task, seq
