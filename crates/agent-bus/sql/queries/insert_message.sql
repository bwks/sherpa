INSERT INTO message (seq, at_ms, task, from_agent, to_agent, kind, text, model, github, usage, task_usage)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
ON CONFLICT (seq) DO NOTHING
