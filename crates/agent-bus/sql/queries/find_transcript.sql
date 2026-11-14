-- Where reading the transcript ?1 stopped.
SELECT inode, offset, started_at_ms, skipped FROM host_transcript WHERE path = ?1
