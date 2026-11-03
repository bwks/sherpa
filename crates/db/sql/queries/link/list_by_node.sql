SELECT id, "index", kind, node_a, node_b, int_a, int_b, lab, tap_a, tap_b, a_to_b_delay_us,
    a_to_b_jitter_us, a_to_b_loss_percent, a_to_b_reorder_percent, a_to_b_corrupt_percent,
    b_to_a_delay_us, b_to_a_jitter_us, b_to_a_loss_percent, b_to_a_reorder_percent,
    b_to_a_corrupt_percent
FROM link
WHERE node_a = ?1 OR node_b = ?1
