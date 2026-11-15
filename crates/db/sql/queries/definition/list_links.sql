SELECT position, src, dst, p2p, impaired, delay, jitter, loss_percent, reorder_percent, corrupt_percent
FROM manifest_link
WHERE lab = ?1
ORDER BY position
