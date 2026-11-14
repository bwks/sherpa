-- Read every transcript of the project ?1 from the start again.
UPDATE host_transcript SET offset = 0 WHERE project = ?1
