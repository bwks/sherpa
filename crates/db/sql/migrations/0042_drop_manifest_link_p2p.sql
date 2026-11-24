-- A manifest link no longer has a `p2p` key (#821): every link is a P2p (eBPF) link, and the
-- manifest refuses the key as unknown. The stored value is dropped with the column.
ALTER TABLE manifest_link DROP COLUMN p2p;
