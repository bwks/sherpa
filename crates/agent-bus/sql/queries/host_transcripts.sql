-- Every transcript read, the latest first.
SELECT path, session, agent_id, offset, imported_at_ms, skipped
FROM host_transcript
ORDER BY imported_at_ms DESC, path
