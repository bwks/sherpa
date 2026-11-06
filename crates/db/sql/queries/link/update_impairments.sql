UPDATE link
SET a_to_b_delay_us = ?2, a_to_b_jitter_us = ?3, a_to_b_loss_percent = ?4,
    a_to_b_reorder_percent = ?5, a_to_b_corrupt_percent = ?6, b_to_a_delay_us = ?7,
    b_to_a_jitter_us = ?8, b_to_a_loss_percent = ?9, b_to_a_reorder_percent = ?10,
    b_to_a_corrupt_percent = ?11
WHERE id = ?1
RETURNING id, "index", kind, node_a, node_b, int_a, int_b, lab, tap_a, tap_b, a_to_b_delay_us,
    a_to_b_jitter_us, a_to_b_loss_percent, a_to_b_reorder_percent, a_to_b_corrupt_percent,
    b_to_a_delay_us, b_to_a_jitter_us, b_to_a_loss_percent, b_to_a_reorder_percent,
    b_to_a_corrupt_percent
