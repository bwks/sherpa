-- The host's active item is ?3 (NULL: none) from ?1 on, in the session ?2 (NULL: every
-- session), because of ?4 (with the task ?5 it tagged), naming the pull request ?6#?7 when not
-- NULL.
INSERT INTO host_focus (from_ms, session, work, source, task, pr_repo, pr_number)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
