-- Keep how writing the section of pull request ?2 in ?1 went.
INSERT INTO generation_pr (repo, number, work, state, section, written_at_ms, last_error)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
ON CONFLICT (repo, number) DO UPDATE SET
    work = excluded.work,
    state = excluded.state,
    section = excluded.section,
    written_at_ms = excluded.written_at_ms,
    last_error = excluded.last_error
