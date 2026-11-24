SELECT position, src, dst, impaired, delay, jitter, loss_percent, reorder_percent, corrupt_percent
FROM manifest_link
WHERE lab = ?1
ORDER BY position
