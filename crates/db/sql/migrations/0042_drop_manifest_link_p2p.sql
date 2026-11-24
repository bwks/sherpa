-- Drop the unused `p2p` flag from manifest links.
ALTER TABLE manifest_link DROP COLUMN p2p;
