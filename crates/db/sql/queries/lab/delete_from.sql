DELETE FROM lab
WHERE lab_id = ?1 AND phase = ?2
RETURNING id
