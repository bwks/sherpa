UPDATE link
SET "index" = ?2, kind = ?3, node_a = ?4, node_b = ?5, int_a = ?6, int_b = ?7, lab = ?8, tap_a = ?9,
    tap_b = ?10, a_to_b_delay_us = ?11, a_to_b_jitter_us = ?12, a_to_b_loss_percent = ?13,
    a_to_b_reorder_percent = ?14, a_to_b_corrupt_percent = ?15, b_to_a_delay_us = ?16,
    b_to_a_jitter_us = ?17, b_to_a_loss_percent = ?18, b_to_a_reorder_percent = ?19,
    b_to_a_corrupt_percent = ?20
WHERE id = ?1
RETURNING id, "index", kind, node_a, node_b, int_a, int_b, lab, tap_a, tap_b, a_to_b_delay_us,
    a_to_b_jitter_us, a_to_b_loss_percent, a_to_b_reorder_percent, a_to_b_corrupt_percent,
    b_to_a_delay_us, b_to_a_jitter_us, b_to_a_loss_percent, b_to_a_reorder_percent,
    b_to_a_corrupt_percent
