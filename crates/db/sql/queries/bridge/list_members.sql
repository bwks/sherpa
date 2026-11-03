SELECT node, interface
FROM bridge_member
WHERE bridge = ?1
ORDER BY position
