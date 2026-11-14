-- Every pull request whose section the host has tried to write.
SELECT repo, number, work, state, section, written_at_ms, last_error, attempted_at_ms
FROM generation_pr
