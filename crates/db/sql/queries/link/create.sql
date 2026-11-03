INSERT INTO link (id, "index", kind, node_a, node_b, int_a, int_b, lab, tap_a, tap_b,
    a_to_b_delay_us, a_to_b_jitter_us, a_to_b_loss_percent, a_to_b_reorder_percent,
    a_to_b_corrupt_percent, b_to_a_delay_us, b_to_a_jitter_us, b_to_a_loss_percent,
    b_to_a_reorder_percent, b_to_a_corrupt_percent)
VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
RETURNING id, "index", kind, node_a, node_b, int_a, int_b, lab, tap_a, tap_b, a_to_b_delay_us,
    a_to_b_jitter_us, a_to_b_loss_percent, a_to_b_reorder_percent, a_to_b_corrupt_percent,
    b_to_a_delay_us, b_to_a_jitter_us, b_to_a_loss_percent, b_to_a_reorder_percent,
    b_to_a_corrupt_percent
