SELECT user.username
FROM lab
JOIN user ON user.id = lab.user
WHERE lab.lab_id = ?1
